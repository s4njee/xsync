# Homebrew formula for xsync. Story D4.2.
#
# Lives here as the source of truth; the release workflow renders it with the
# published version and checksums and pushes it to the tap. Keeping the template
# in this repository means the formula is reviewed alongside the code it
# installs rather than drifting in a separate repo nobody watches.
class Xsync < Formula
  desc "High-performance rsync replacement built on a parallel pipeline and BLAKE3"
  homepage "https://github.com/s4njee/xsync"
  version "__VERSION__"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/s4njee/xsync/releases/download/v#{version}/xsync-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "__SHA256_AARCH64_APPLE_DARWIN__"
    end
    on_intel do
      url "https://github.com/s4njee/xsync/releases/download/v#{version}/xsync-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "__SHA256_X86_64_APPLE_DARWIN__"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/s4njee/xsync/releases/download/v#{version}/xsync-#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "__SHA256_AARCH64_UNKNOWN_LINUX_GNU__"
    end
    on_intel do
      url "https://github.com/s4njee/xsync/releases/download/v#{version}/xsync-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "__SHA256_X86_64_UNKNOWN_LINUX_GNU__"
    end
  end

  def install
    bin.install "xs"
    man1.install "man/xs.1" if File.exist?("man/xs.1")
    bash_completion.install "completions/xs.bash" => "xs" if File.exist?("completions/xs.bash")
    zsh_completion.install "completions/_xs" if File.exist?("completions/_xs")
    fish_completion.install "completions/xs.fish" if File.exist?("completions/xs.fish")
  end

  test do
    # Assert the binary reports the version Homebrew installed. A smoke test
    # that only checks the process exits zero would pass on a stale binary.
    assert_match version.to_s, shell_output("#{bin}/xs -V")
    (testpath/"src").mkpath
    (testpath/"src/a.txt").write "hello"
    system bin/"xs", "#{testpath}/src/", "#{testpath}/dst"
    assert_equal "hello", (testpath/"dst/a.txt").read
  end
end
