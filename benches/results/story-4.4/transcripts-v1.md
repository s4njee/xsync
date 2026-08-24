# Normalized Rsync Golden Transcripts v1

These are semantic golden transcripts, not raw byte dumps. They normalize
implementation debug text, path roots, and the random checksum seed while
retaining protocol order, flags, indexes, checksums, errors, and exit status.
Raw-byte captures belong in the Story 4.5 codec tests once the reference
oracle harness exists.

`proto` is the negotiated protocol. `<seed>` must be a fresh signed 32-bit
value in every non-replayed session. `path` values are raw byte sequences;
`hex:` is used for the non-UTF-8 fixture.

## GNU protocol 32

```text
scenario=handshake
client.send version=32
server.send version=32
negotiated proto=32
server.send checksum_seed=<seed>
result=ready

scenario=regular-file
setup proto=32 whole_file=true
file_list ["hello.txt"] attrs={kind=file,size=6,mtime=<normalized>,mode=0644}
file_list.end
update index=0 phase=1
literal index=0 bytes=6
whole_file_checksum index=0 algorithm=<negotiated> value=<digest>
result=exit=0

scenario=nested-tree-and-empty-directory
file_list ["empty/", "nested/", "nested/value.txt"]
directory.create "empty/"
directory.create "nested/"
literal path="nested/value.txt" bytes=<n>
metadata.apply paths=["empty/","nested/","nested/value.txt"]
result=exit=0

scenario=symlink
file_list ["link"] attrs={kind=symlink,target="nested/value.txt"}
symlink.create path="link" target="nested/value.txt"
result=exit=0

scenario=non-utf8-unix-name
file_list [hex:6e616d65ff] attrs={kind=file,size=1}
literal path=hex:6e616d65ff bytes=1
result=exit=0

scenario=metadata
file_list ["meta.txt"] attrs={kind=file,mode=0600,mtime=<normalized>}
metadata.apply path="meta.txt" mode=0600 mtime=<normalized>
result=exit=0

scenario=unchanged-file
file_list ["same.txt"] attrs={kind=file,size=<same>,mtime=<same>}
update index=0 action=skip
result=exit=0

scenario=receiver-error
server.multiplex tag=1 message="permission denied"
result=error=remote-exit-nonzero

scenario=clean-end
phase.change sender=1 receiver=1
phase.change sender=2 receiver=2
phase.change sender=3 receiver=3
stream.eof only_after=protocol-complete
result=exit=0
```

## Apple openrsync protocol 29 boundary

The protocol-29 fixture uses the same semantic scenarios, but records the
known Apple dialect boundary: legacy fixed-width file-list flags and no
protocol-32 negotiated checksum/compression strings. The handshake below was
observed from the local Apple client against the GNU protocol-32 receiver with
`--protocol=29`; the file-list and update stages remain the normalized
receiver contract for Story 4.5.

```text
scenario=handshake
client.send version=29
server.send version=32
negotiated proto=29
server.send checksum_seed=<seed>
result=ready

scenario=regular-file
file_list legacy_flags=true ["hello.txt"]
literal index=0 bytes=6
whole_file_checksum index=0 algorithm=md4-or-dialect-choice value=<digest>
result=exit=0

scenario=nested-tree-and-empty-directory
file_list legacy_flags=true ["empty/","nested/","nested/value.txt"]
result=exit=0

scenario=symlink
file_list ["link"] attrs={kind=symlink,target="nested/value.txt"}
result=exit=0

scenario=non-utf8-unix-name
file_list [hex:6e616d65ff]
result=exit=0

scenario=metadata
metadata.apply path="meta.txt" mode=0600 mtime=<normalized>
result=exit=0

scenario=unchanged-file
update index=0 action=skip
result=exit=0

scenario=receiver-error
server.multiplex tag=1 message="permission denied"
result=error=remote-exit-nonzero

scenario=clean-end
phase.change acknowledgement=required-for-proto-29
stream.eof only_after=protocol-complete
result=exit=0
```
