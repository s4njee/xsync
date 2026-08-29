//! Ordered include/exclude rules (Story V3.10).
//!
//! # Why this is not a copy of rsync
//!
//! rsync's filter rules are widely considered its most confusing surface, for
//! three separate reasons. This module keeps the part that is genuinely good
//! and fixes the rest:
//!
//! 1. **First matching rule wins, in the order given.** Kept — it is simple to
//!    state and simple to reason about, and it is what people who know rsync
//!    already expect. The order is the order the rules were written on the
//!    command line, including across `--include`, `--exclude`, `--include-from`
//!    and `--exclude-from`.
//! 2. **rsync makes you write `--include '*/'` to descend into directories.**
//!    Dropped. Forgetting it is the single most common way an rsync include
//!    rule silently matches nothing, because the directory holding the wanted
//!    files was pruned before the rule was ever consulted. Here a directory is
//!    descended whenever an include rule *could* match something beneath it,
//!    computed from the rules themselves.
//! 3. **rsync's per-directory `.rsync-filter` files interleave with
//!    command-line rules by position.** Simplified: `.xsyncignore` files are
//!    always weaker than command-line rules, so a command line can always
//!    override a tree's own opinion, and never the other way round.
//!
//! # Evaluation
//!
//! A path is tested against every rule in order; the first that matches decides.
//! When nothing matches the path is **included** — the default is to transfer,
//! and rules subtract from that.
//!
//! Patterns are `globset` globs matched against the path *relative to the scan
//! root*, so `**` crosses directory boundaries. A path is also excluded when any
//! of its ancestor directories is excluded, which is what makes pruning and
//! per-path evaluation agree.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

use globset::{Glob, GlobMatcher};

/// The name of the per-directory ignore file.
pub const IGNORE_FILE_NAME: &str = ".xsyncignore";

/// What a rule does when it matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Transfer the path, ending evaluation.
    Include,
    /// Skip the path, ending evaluation.
    Exclude,
}

impl Action {
    /// The single character used for this action on the wire and in messages.
    #[must_use]
    pub const fn sigil(self) -> char {
        match self {
            Self::Include => '+',
            Self::Exclude => '-',
        }
    }
}

/// Where a rule came from, so a decision can be explained precisely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A `--include` or `--exclude` flag, at this position on the command line.
    CommandLine,
    /// A line in a `--include-from` or `--exclude-from` file.
    File {
        /// The file the rule was read from.
        path: String,
        /// One-based line number within that file.
        line: usize,
    },
    /// A line in a per-directory ignore file.
    IgnoreFile {
        /// The ignore file's path, relative to the scan root.
        path: String,
        /// One-based line number within that file.
        line: usize,
    },
    /// A rule reconstructed from the wire, whose origin lives on the far side.
    Remote,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CommandLine => write!(formatter, "command line"),
            // A rules file and an ignore file are both named the same way —
            // `path:line` — because that is what a reader needs in order to go
            // and edit the offending rule. The variants stay distinct so a
            // caller can still tell which tier a decision came from.
            Self::File { path, line } | Self::IgnoreFile { path, line } => {
                write!(formatter, "{path}:{line}")
            }
            Self::Remote => write!(formatter, "sent by the client"),
        }
    }
}

/// One include or exclude rule.
#[derive(Debug, Clone)]
pub struct Rule {
    /// What happens when this rule matches.
    pub action: Action,
    /// The glob as written, kept for messages and for the wire.
    pub pattern: String,
    /// Where the rule was written.
    pub origin: Origin,
    matcher: GlobMatcher,
    /// Globs matching the directories that must be descended for this rule's
    /// pattern to have any chance of matching. Empty for exclude rules.
    descend: Vec<GlobMatcher>,
    /// True when the pattern can match at any depth, so every directory must be
    /// descended.
    descend_anywhere: bool,
}

impl PartialEq for Rule {
    fn eq(&self, other: &Self) -> bool {
        self.action == other.action && self.pattern == other.pattern
    }
}

impl Eq for Rule {}

/// A rule's pattern was not a usable glob.
#[derive(Debug, thiserror::Error)]
pub enum FilterError {
    /// The glob could not be compiled.
    #[error("invalid filter pattern '{pattern}' ({origin}): {message}")]
    Pattern {
        /// The pattern as written.
        pattern: String,
        /// Where it came from.
        origin: Origin,
        /// The glob compiler's message.
        message: String,
    },
    /// A rules file could not be read.
    #[error("cannot read filter file '{path}': {source}")]
    Unreadable {
        /// The file that could not be read.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A wire-encoded rule did not carry a recognized action.
    #[error("malformed filter rule from peer: {0}")]
    Malformed(String),
}

impl Rule {
    /// Compile one rule.
    ///
    /// # Errors
    ///
    /// Returns [`FilterError::Pattern`] when `pattern` is not a valid glob.
    pub fn new(action: Action, pattern: &str, origin: Origin) -> Result<Self, FilterError> {
        let glob = Glob::new(pattern).map_err(|error| FilterError::Pattern {
            pattern: pattern.to_owned(),
            origin: origin.clone(),
            message: error.to_string(),
        })?;
        let (descend, descend_anywhere) = if action == Action::Include {
            descend_globs(pattern)?
        } else {
            (Vec::new(), false)
        };
        Ok(Self {
            action,
            pattern: pattern.to_owned(),
            origin,
            matcher: glob.compile_matcher(),
            descend,
            descend_anywhere,
        })
    }

    fn matches(&self, relative: &str) -> bool {
        self.matcher.is_match(relative)
    }
}

/// Build the set of directory globs that must be entered for `pattern` to have
/// a chance of matching something beneath them.
///
/// For `docs/api/*.md` these are `docs` and `docs/api`. A pattern containing
/// `**` can match at any depth, which is reported separately so the caller can
/// stop trying to be clever and descend everything.
///
/// This over-approximates on purpose: descending a directory that turns out to
/// hold nothing wanted costs a readdir, while failing to descend one loses
/// files silently.
fn descend_globs(pattern: &str) -> Result<(Vec<GlobMatcher>, bool), FilterError> {
    if pattern.contains("**") {
        return Ok((Vec::new(), true));
    }
    let components: Vec<&str> = pattern.split('/').collect();
    let mut matchers = Vec::new();
    for depth in 1..components.len() {
        let prefix = components[..depth].join("/");
        let glob = Glob::new(&prefix).map_err(|error| FilterError::Pattern {
            pattern: prefix.clone(),
            origin: Origin::CommandLine,
            message: error.to_string(),
        })?;
        matchers.push(glob.compile_matcher());
    }
    Ok((matchers, false))
}

/// What a filter decided about one path, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// Whether the path is transferred.
    pub action: Action,
    /// The rule that decided, or `None` when nothing matched and the default
    /// (include) applied.
    pub rule: Option<Rule>,
    /// The ancestor directory whose exclusion carried the path, when the path
    /// itself matched nothing.
    pub via_ancestor: Option<String>,
}

impl Decision {
    /// Whether this decision transfers the path.
    #[must_use]
    pub const fn is_included(&self) -> bool {
        matches!(self.action, Action::Include)
    }

    /// A one-line explanation suitable for `--explain-filter`.
    #[must_use]
    pub fn explain(&self) -> String {
        let Some(rule) = &self.rule else {
            return "no rule matched; included by default".to_owned();
        };
        let verb = match rule.action {
            Action::Include => "included",
            Action::Exclude => "excluded",
        };
        match &self.via_ancestor {
            Some(ancestor) => format!(
                "{verb} by '{} {}' ({}), which excluded the parent '{ancestor}'",
                rule.action.sigil(),
                rule.pattern,
                rule.origin
            ),
            None => format!(
                "{verb} by '{} {}' ({})",
                rule.action.sigil(),
                rule.pattern,
                rule.origin
            ),
        }
    }
}

/// Per-directory `.xsyncignore` rules, discovered during the walk.
///
/// This is a second, weaker tier than the command-line rules: a command line can
/// always override a tree's own opinion, never the other way round. Getting that
/// direction right is the reason this is not delegated to the `ignore` crate's
/// own custom-ignore support, which prunes during the walk and therefore beats
/// every rule the user typed — and cannot be explained afterwards, because a
/// pruned path is never seen again.
///
/// Rules are keyed by the directory that contained the file, as a `/`-separated
/// path relative to the scan root (`""` for the root itself), and match against
/// paths relative to *that* directory, the way `.gitignore` does.
#[derive(Debug, Default)]
pub struct IgnoreLayer {
    directories: RwLock<HashMap<String, Vec<Rule>>>,
}

impl IgnoreLayer {
    /// Load `directory`'s ignore file, if it has one.
    ///
    /// Called by the scanner as each directory is entered, which the walker
    /// guarantees happens before any of that directory's children are yielded.
    ///
    /// # Errors
    ///
    /// Returns [`FilterError::Pattern`] when a line is not a valid glob. An
    /// unreadable ignore file is reported so it cannot silently stop applying.
    pub fn load(&self, directory: &Path, relative: &str) -> Result<(), FilterError> {
        let path = directory.join(IGNORE_FILE_NAME);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(FilterError::Unreadable {
                    path: path.display().to_string(),
                    source,
                })
            }
        };
        let display = if relative.is_empty() {
            IGNORE_FILE_NAME.to_owned()
        } else {
            format!("{relative}/{IGNORE_FILE_NAME}")
        };
        let mut rules = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let origin = Origin::IgnoreFile {
                path: display.clone(),
                line: index + 1,
            };
            if let Some(rule) = rule_from_line(line, Action::Exclude, origin)? {
                rules.push(rule);
            }
        }
        if let Ok(mut directories) = self.directories.write() {
            directories.insert(relative.to_owned(), rules);
        }
        Ok(())
    }

    /// Decide `relative` against the ignore files that govern it.
    ///
    /// The nearest enclosing directory's rules are consulted first, so a nested
    /// `.xsyncignore` overrides one further up.
    fn decide(&self, relative: &str) -> Option<Rule> {
        let directories = self.directories.read().ok()?;
        if directories.is_empty() {
            return None;
        }
        let mut scope = relative;
        loop {
            let (parent, _) = scope.rsplit_once('/').unwrap_or(("", scope));
            // A directory with no ignore file of its own is not the end of the
            // search: the enclosing directories still have a say, exactly as
            // with `.gitignore`. Stopping here was a bug that made a root rule
            // apply to root-level entries only.
            if let Some(rules) = directories.get(parent) {
                // Patterns match against the path relative to the directory
                // holding the ignore file, which is what makes a rule written
                // in a subdirectory mean what its author expects.
                let local = if parent.is_empty() {
                    relative
                } else {
                    relative
                        .strip_prefix(parent)
                        .unwrap_or(relative)
                        .trim_start_matches('/')
                };
                for rule in rules {
                    if rule.matches(local) {
                        return Some(rule.clone());
                    }
                }
            }
            if parent.is_empty() {
                return None;
            }
            scope = parent;
        }
    }
}

/// An ordered set of rules, evaluated first-match-wins.
#[derive(Debug, Clone, Default)]
pub struct FilterSet {
    rules: Vec<Rule>,
    /// Whether per-directory ignore files are honoured during the scan.
    ignore_files: bool,
    ignore: Option<Arc<IgnoreLayer>>,
    /// Load ignore files but never act on them, so a walk sees everything.
    observing: bool,
}

impl FilterSet {
    /// An empty filter that includes everything.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from an ordered rule list.
    #[must_use]
    pub fn from_rules(rules: Vec<Rule>) -> Self {
        Self {
            rules,
            ignore_files: false,
            ignore: None,
            observing: false,
        }
    }

    /// Honour `.xsyncignore` files encountered during the scan.
    #[must_use]
    pub fn with_ignore_files(mut self, enabled: bool) -> Self {
        self.ignore_files = enabled;
        self.ignore = enabled.then(|| Arc::new(IgnoreLayer::default()));
        self
    }

    /// The per-directory ignore rules discovered so far, if enabled.
    #[must_use]
    pub fn ignore_layer(&self) -> Option<&Arc<IgnoreLayer>> {
        self.ignore.as_ref()
    }

    /// A rule-free filter sharing this one's ignore layer.
    ///
    /// Used to walk a tree without pruning while still discovering its ignore
    /// files, so that `--explain-filter` can name the rule that would have
    /// removed a path. Sharing the layer is the point: a fresh one would be
    /// populated by the walk and then thrown away, leaving the real filter
    /// blind to every ignore file in the tree.
    #[must_use]
    pub fn observing_only(&self) -> Self {
        Self {
            rules: Vec::new(),
            ignore_files: self.ignore_files,
            ignore: self.ignore.clone(),
            // Discovering an ignore file must not also obey it here: obeying it
            // would prune the very paths the caller is trying to have explained.
            observing: true,
        }
    }

    /// Whether per-directory ignore files are honoured.
    #[must_use]
    pub const fn honours_ignore_files(&self) -> bool {
        self.ignore_files
    }

    /// The rules, in evaluation order.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Whether this filter can affect anything at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty() && !self.ignore_files
    }

    /// Whether any rule is an include.
    ///
    /// Include rules are what make a filter inexpressible to a peer that only
    /// understands a flat exclude list, so this drives the fail-closed check.
    #[must_use]
    pub fn has_includes(&self) -> bool {
        self.rules.iter().any(|rule| rule.action == Action::Include)
    }

    /// Decide one path, given as a `/`-separated path relative to the scan root.
    #[must_use]
    pub fn decide(&self, relative: &str) -> Decision {
        for rule in &self.rules {
            if rule.matches(relative) {
                return Decision {
                    action: rule.action,
                    rule: Some(rule.clone()),
                    via_ancestor: None,
                };
            }
        }
        // A path under an excluded directory is excluded even when the path
        // itself matches nothing, so that per-path evaluation and directory
        // pruning cannot disagree.
        let mut prefix = relative;
        while let Some((ancestor, _)) = prefix.rsplit_once('/') {
            for rule in &self.rules {
                if rule.matches(ancestor) {
                    return Decision {
                        action: rule.action,
                        rule: Some(rule.clone()),
                        via_ancestor: Some(ancestor.to_owned()),
                    };
                }
            }
            prefix = ancestor;
        }
        // Only now, with every command-line rule silent, does the tree get a
        // say. Its own ancestors are walked the same way, so an ignored
        // directory carries its contents with it.
        if let Some(ignore) = self.ignore.as_ref().filter(|_| !self.observing) {
            if let Some(rule) = ignore.decide(relative) {
                return Decision {
                    action: rule.action,
                    rule: Some(rule),
                    via_ancestor: None,
                };
            }
            let mut prefix = relative;
            while let Some((ancestor, _)) = prefix.rsplit_once('/') {
                if let Some(rule) = ignore.decide(ancestor) {
                    return Decision {
                        action: rule.action,
                        rule: Some(rule),
                        via_ancestor: Some(ancestor.to_owned()),
                    };
                }
                prefix = ancestor;
            }
        }
        Decision {
            action: Action::Include,
            rule: None,
            via_ancestor: None,
        }
    }

    /// Whether a directory must be walked.
    ///
    /// An excluded directory is still walked when an include rule could match
    /// something beneath it. This is the `--include '*/'` footgun removed: in
    /// rsync, `--exclude '*' --include 'docs/**'` matches nothing, because `docs`
    /// is pruned before the include rule is ever reached.
    #[must_use]
    pub fn should_descend(&self, relative: &str) -> bool {
        if self.decide(relative).is_included() {
            return true;
        }
        self.rules.iter().any(|rule| {
            rule.action == Action::Include
                && (rule.descend_anywhere
                    || rule.descend.iter().any(|glob| glob.is_match(relative)))
        })
    }
}

/// Read filter rules from a file, one per line.
///
/// Blank lines and lines whose first non-space character is `#` are ignored, so
/// a rules file can carry comments. Every rule in the file takes `action`, which
/// is what distinguishes `--include-from` from `--exclude-from`; a line may
/// still override it with a leading `+ ` or `- `, matching the wire encoding and
/// the ignore-file format so there is only one syntax to learn.
///
/// # Errors
///
/// Returns [`FilterError::Unreadable`] when the file cannot be read, and
/// [`FilterError::Pattern`] when a line is not a valid glob.
pub fn rules_from_file(path: &Path, action: Action) -> Result<Vec<Rule>, FilterError> {
    let text = std::fs::read_to_string(path).map_err(|source| FilterError::Unreadable {
        path: path.display().to_string(),
        source,
    })?;
    let display = path.display().to_string();
    let mut rules = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let origin = Origin::File {
            path: display.clone(),
            line: index + 1,
        };
        if let Some(rule) = rule_from_line(line, action, origin)? {
            rules.push(rule);
        }
    }
    Ok(rules)
}

/// Parse one line of a rules or ignore file.
///
/// Returns `Ok(None)` for a blank line or a comment.
///
/// # Errors
///
/// Returns [`FilterError::Pattern`] when the line is not a valid glob.
pub fn rule_from_line(
    line: &str,
    default_action: Action,
    origin: Origin,
) -> Result<Option<Rule>, FilterError> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Ok(None);
    }
    let (action, pattern) = if let Some(rest) = trimmed.strip_prefix("+ ") {
        (Action::Include, rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix("- ") {
        (Action::Exclude, rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix('!') {
        // gitignore spells negation `!pattern`, and enough people will reach for
        // it in a `.xsyncignore` that silently treating it as a literal filename
        // starting with '!' would be a trap.
        (Action::Include, rest.trim())
    } else {
        (default_action, trimmed)
    };
    if pattern.is_empty() {
        return Ok(None);
    }
    Rule::new(action, pattern, origin).map(Some)
}

/// Encode a filter for the wire, one entry per rule.
///
/// The encoding is `"<sigil> <pattern>"`, which only a peer advertising
/// [`crate::protocol::CAP_FILTER_RULES`] is asked to parse. A peer without that
/// capability is never sent a filter it cannot represent — see
/// [`FilterSet::has_includes`].
#[must_use]
pub fn encode(filter: &FilterSet) -> Vec<Vec<u8>> {
    filter
        .rules()
        .iter()
        .map(|rule| format!("{} {}", rule.action.sigil(), rule.pattern).into_bytes())
        .collect()
}

/// Decode a wire-encoded filter.
///
/// # Errors
///
/// Returns [`FilterError::Malformed`] when an entry carries no recognized
/// action sigil, and [`FilterError::Pattern`] when a pattern is not a valid
/// glob. Both are fail-closed: a filter that cannot be understood exactly must
/// never be approximated, because the approximation silently transfers or
/// silently skips.
pub fn decode(entries: &[Vec<u8>]) -> Result<FilterSet, FilterError> {
    let mut rules = Vec::new();
    for entry in entries {
        let text = std::str::from_utf8(entry)
            .map_err(|_| FilterError::Malformed("rule is not valid UTF-8".to_owned()))?;
        let (action, pattern) = if let Some(rest) = text.strip_prefix("+ ") {
            (Action::Include, rest)
        } else if let Some(rest) = text.strip_prefix("- ") {
            (Action::Exclude, rest)
        } else {
            return Err(FilterError::Malformed(format!(
                "rule '{text}' has no '+ ' or '- ' prefix"
            )));
        };
        rules.push(Rule::new(action, pattern, Origin::Remote)?);
    }
    Ok(FilterSet::from_rules(rules))
}

/// Compile a legacy flat exclude list into a filter.
///
/// # Errors
///
/// Returns [`FilterError::Pattern`] when a pattern is not a valid glob.
pub fn from_exclude_patterns(patterns: &[String]) -> Result<FilterSet, FilterError> {
    let mut rules = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        rules.push(Rule::new(Action::Exclude, pattern, Origin::CommandLine)?);
    }
    Ok(FilterSet::from_rules(rules))
}

/// A filter shared with the scanner's walker threads.
pub type SharedFilter = Arc<FilterSet>;

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(rules: &[(Action, &str)]) -> FilterSet {
        FilterSet::from_rules(
            rules
                .iter()
                .map(|(action, pattern)| Rule::new(*action, pattern, Origin::CommandLine).unwrap())
                .collect(),
        )
    }

    fn ignore_tree(files: &[(&str, &str)]) -> (tempfile::TempDir, FilterSet) {
        let dir = tempfile::tempdir().unwrap();
        for (relative, contents) in files {
            let path = dir.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        let filter = FilterSet::new().with_ignore_files(true);
        (dir, filter)
    }

    #[test]
    fn an_ignore_file_applies_below_its_own_directory() {
        let (dir, filter) = ignore_tree(&[(".xsyncignore", "*.log\ntarget\n")]);
        filter.ignore_layer().unwrap().load(dir.path(), "").unwrap();

        assert!(!filter.decide("build.log").is_included());
        // The bug this pins: a rule at the root must reach a nested path, even
        // through directories that have no ignore file of their own.
        assert!(!filter.decide("logs/build.log").is_included());
        assert!(!filter.decide("target/debug/app").is_included());
        assert!(filter.decide("src/main.rs").is_included());
    }

    #[test]
    fn a_command_line_rule_overrides_the_tree() {
        // The direction that matters: a command line can override a tree's own
        // opinion, never the other way round.
        let (dir, base) = ignore_tree(&[(".xsyncignore", "*.log\n")]);
        base.ignore_layer().unwrap().load(dir.path(), "").unwrap();

        let mut filter = FilterSet::from_rules(vec![Rule::new(
            Action::Include,
            "logs/**",
            Origin::CommandLine,
        )
        .unwrap()])
        .with_ignore_files(true);
        // Share the layer the fixture populated.
        filter = FilterSet {
            rules: filter.rules().to_vec(),
            ignore_files: true,
            ignore: base.ignore_layer().cloned(),
            observing: false,
        };
        assert!(filter.decide("logs/build.log").is_included());
        assert!(
            !filter.decide("other.log").is_included(),
            "elsewhere it still applies"
        );
    }

    #[test]
    fn a_nested_ignore_file_matches_relative_to_its_own_directory() {
        let (dir, filter) = ignore_tree(&[
            (".xsyncignore", "top.txt\n"),
            ("sub/.xsyncignore", "local.txt\n"),
        ]);
        let layer = filter.ignore_layer().unwrap();
        layer.load(dir.path(), "").unwrap();
        layer.load(&dir.path().join("sub"), "sub").unwrap();

        assert!(!filter.decide("sub/local.txt").is_included());
        // `local.txt` is written in `sub/.xsyncignore`, so it means
        // `sub/local.txt` and not a file of that name at the root.
        assert!(filter.decide("local.txt").is_included());
        assert!(!filter.decide("top.txt").is_included());
    }

    #[test]
    fn an_ignore_decision_names_the_file_and_line() {
        let (dir, filter) = ignore_tree(&[(".xsyncignore", "# comment\n*.log\n")]);
        filter.ignore_layer().unwrap().load(dir.path(), "").unwrap();
        let message = filter.decide("build.log").explain();
        assert!(message.contains(".xsyncignore:2"), "{message}");
    }

    #[test]
    fn an_observing_filter_discovers_ignore_files_without_obeying_them() {
        let (dir, filter) = ignore_tree(&[(".xsyncignore", "*.log\n")]);
        let observer = filter.observing_only();
        observer
            .ignore_layer()
            .unwrap()
            .load(dir.path(), "")
            .unwrap();

        assert!(
            observer.decide("build.log").is_included(),
            "the observing walk must see everything it is meant to explain"
        );
        assert!(
            !filter.decide("build.log").is_included(),
            "while the real filter, sharing the layer, still excludes it"
        );
    }

    #[test]
    fn nothing_matching_is_included() {
        let filter = filter(&[(Action::Exclude, "*.tmp")]);
        assert!(filter.decide("notes.txt").is_included());
        assert!(filter.decide("notes.txt").rule.is_none());
    }

    #[test]
    fn the_first_matching_rule_wins() {
        // Both rules match; order alone decides, in each direction.
        let keep = filter(&[(Action::Include, "*.log"), (Action::Exclude, "*.log")]);
        assert!(keep.decide("build.log").is_included());

        let drop = filter(&[(Action::Exclude, "*.log"), (Action::Include, "*.log")]);
        assert!(!drop.decide("build.log").is_included());
    }

    #[test]
    fn an_include_before_a_broad_exclude_rescues_one_path() {
        let filter = filter(&[(Action::Include, "keep/**"), (Action::Exclude, "**")]);
        assert!(filter.decide("keep/a.txt").is_included());
        assert!(!filter.decide("other/a.txt").is_included());
    }

    #[test]
    fn a_path_under_an_excluded_directory_is_excluded() {
        let filter = filter(&[(Action::Exclude, "target")]);
        let decision = filter.decide("target/debug/app");
        assert!(!decision.is_included());
        assert_eq!(decision.via_ancestor.as_deref(), Some("target"));
    }

    #[test]
    fn an_included_directory_is_descended() {
        let filter = filter(&[(Action::Exclude, "*.tmp")]);
        assert!(filter.should_descend("docs"));
    }

    #[test]
    fn an_excluded_directory_is_still_descended_for_an_include_beneath_it() {
        // The rsync footgun this design removes: with `--exclude '*'` alone,
        // rsync prunes `docs` and the include rule never runs.
        let filter = filter(&[(Action::Include, "docs/api/*.md"), (Action::Exclude, "*")]);
        assert!(
            !filter.decide("docs").is_included(),
            "docs itself is excluded"
        );
        assert!(filter.should_descend("docs"), "but it must still be walked");
        assert!(filter.should_descend("docs/api"));
        assert!(filter.decide("docs/api/ref.md").is_included());
        assert!(!filter.should_descend("src"), "unrelated trees stay pruned");
    }

    #[test]
    fn a_double_star_include_descends_everywhere() {
        let filter = filter(&[(Action::Include, "**/*.md"), (Action::Exclude, "*")]);
        assert!(filter.should_descend("anything"));
        assert!(filter.should_descend("deeply/nested/tree"));
    }

    #[test]
    fn an_empty_filter_includes_and_descends_everything() {
        let filter = FilterSet::new();
        assert!(filter.is_empty());
        assert!(filter.decide("anything/at/all").is_included());
        assert!(filter.should_descend("anything"));
    }

    #[test]
    fn a_decision_explains_itself() {
        let filter = FilterSet::from_rules(vec![Rule::new(
            Action::Exclude,
            "*.tmp",
            Origin::File {
                path: "excludes.txt".to_owned(),
                line: 3,
            },
        )
        .unwrap()]);
        let message = filter.decide("scratch.tmp").explain();
        assert!(message.contains("excluded by '- *.tmp'"), "{message}");
        assert!(message.contains("excludes.txt:3"), "{message}");
    }

    #[test]
    fn an_ancestor_decision_names_the_ancestor() {
        let filter = filter(&[(Action::Exclude, "target")]);
        let message = filter.decide("target/debug/app").explain();
        assert!(
            message.contains("excluded the parent 'target'"),
            "{message}"
        );
    }

    #[test]
    fn a_rules_file_line_may_override_the_files_default_action() {
        let rule = rule_from_line("+ keep.txt", Action::Exclude, Origin::CommandLine)
            .unwrap()
            .unwrap();
        assert_eq!(rule.action, Action::Include);
        assert_eq!(rule.pattern, "keep.txt");
    }

    #[test]
    fn gitignore_style_negation_is_understood_rather_than_taken_literally() {
        let rule = rule_from_line("!keep.txt", Action::Exclude, Origin::CommandLine)
            .unwrap()
            .unwrap();
        assert_eq!(rule.action, Action::Include);
        assert_eq!(rule.pattern, "keep.txt");
    }

    #[test]
    fn blank_lines_and_comments_produce_no_rule() {
        for line in ["", "   ", "# a comment", "  # indented comment"] {
            assert!(
                rule_from_line(line, Action::Exclude, Origin::CommandLine)
                    .unwrap()
                    .is_none(),
                "{line:?}"
            );
        }
    }

    #[test]
    fn an_invalid_pattern_names_itself_and_its_origin() {
        let error = Rule::new(
            Action::Exclude,
            "a[",
            Origin::File {
                path: "rules.txt".to_owned(),
                line: 7,
            },
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("a["), "{message}");
        assert!(message.contains("rules.txt:7"), "{message}");
    }

    #[test]
    fn encoding_round_trips_through_the_wire_form() {
        let original = filter(&[(Action::Include, "keep/**"), (Action::Exclude, "*.tmp")]);
        let decoded = decode(&encode(&original)).unwrap();
        assert_eq!(decoded.rules(), original.rules());
        assert!(decoded.has_includes());
    }

    #[test]
    fn a_rule_without_an_action_sigil_is_refused_rather_than_guessed() {
        // Approximating an unparseable filter would silently transfer or
        // silently skip; both are worse than failing.
        let error = decode(&[b"*.tmp".to_vec()]).unwrap_err();
        assert!(error.to_string().contains("no '+ ' or '- ' prefix"));
    }

    #[test]
    fn a_flat_exclude_list_has_no_includes() {
        let filter = from_exclude_patterns(&["*.tmp".to_owned(), "target".to_owned()]).unwrap();
        assert!(!filter.has_includes());
        assert!(!filter.decide("a.tmp").is_included());
    }
}
