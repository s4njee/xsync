# Browse Connection Model

Story 13.3 applies Story 4.3's SSH decision to the long-lived v2 browse
session.

## Connection

- Use one ordinary `ssh host xs --server <root>` process per remote pane.
- Do not create, require, or modify an SSH `ControlMaster` or control socket.
- User SSH configuration, host-key checks, authentication prompts, and agent
  policy remain under OpenSSH's control. xsync does not weaken or duplicate
  them.
- Keep the v2 session open for the lifetime of the pane. Application
  keepalives must be sent more frequently than the deployment's SSH
  `ClientAliveInterval` and any intervening NAT idle timeout, as established by
  Story 10.5.

## Reconnect

The current v2 session has no resumable session identity. A dropped link is
reported as `PeerDisconnected`; the client must establish a new SSH process,
run the handshake/probe again, and recreate the browse session.

The client must redo:

- Any list request, from the first page. Page cursors are scoped to the old
  request/session and are not valid after reconnect.
- Any stat, rename, mkdir, or delete request whose terminal response was not
  received. Mutations are single-shot; the client should inspect the remote
  state before retrying rather than assume whether the syscall completed.
- Any fetch that did not receive and verify its complete digest. Its local
  temporary file is discarded and the fetch restarts.
- Any publish that did not receive a terminal response. The server's atomic
  staging means the remote target is never a partially written file; the client
  must re-fetch identity before retrying to avoid overwriting a newer edit.

## Transfer Isolation

Browsing does not replace or weaken the existing sync transport. An in-flight
sync uses its existing durable checkpoint journal and chunk identity, so a
separate SSH link drop still resumes through Epic 3.4. Browse requests have no
journal because list/stat/mutation operations are not resumable; fetch and
publish use verified temporary files and must be retried as whole requests.
