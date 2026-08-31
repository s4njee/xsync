# Network topology, and which measurements it invalidates

Recorded 2026-08-31, from the operator's description plus direct measurement.

Every cross-host number in this project was taken over one of two physically
different paths, and until now they were reported as if they were the same
"~1 GbE LAN". They are not. **One is a switched segment; the other crosses a
mesh backhaul with a ceiling below gigabit.**

## The layout

```
                        mesh backhaul, ~800 Mbit/s
        router1  <───────────────────────────────>  router2 ──── freya
           │                     │
           │                     └────────────────>  router3 ──── orion
       ethernet switch
        │         │
       Mac       mars (also Windows / WSL2 -- same physical machine, dual boot)
```

- **Mac, mars, and the Windows/WSL2 installs share router1 through an ethernet
  switch.** These are the only genuinely physically-connected routes.
- **freya sits on router2, orion on router3.** Traffic to either crosses the
  mesh backhaul, which the operator rates at ~800 Mbit/s.
- **mars is dual-homed to router1**, by ethernet *and* WiFi. See the warning
  below; this one silently corrupts measurements.

## Measured, 2026-08-31

`dd if=/dev/zero bs=1m count=1000 | ssh <host> 'cat > /dev/null'`, alternated
between hosts in one session:

| route | runs (MB/s) | median | | RTT (avg / stddev) |
|---|---|---|---|---|
| Mac → mars, switched | 113.7 / 110.5 / 113.0 | 113.0 | **904 Mbit/s** | 1.005 ms / 0.198 |
| Mac → freya, mesh | 96.5 / 99.7 / 99.0 | 99.0 | **792 Mbit/s** | 3.904 ms / 1.542 |

The mesh path delivers **87%** of the switched path's throughput, at **3.9x**
the latency and **8x** the jitter. The 792 Mbit/s figure matches the operator's
~800 Mbit/s rating for the backhaul closely enough to treat it as the cap.

orion was powered off at the time and is untested here, but it sits behind the
same class of hop as freya.

## What this qualifies

**Throughput ceilings measured against freya or orion are not gigabit
ceilings.** They are mesh ceilings, ~12% lower, with materially worse latency.
Any conclusion of the form "the link is the limit" drawn on those paths needs
re-reading: the limit was a backhaul, not an ethernet segment.

**Small-file conclusions largely survive.** congress-100k moves ~144 MB of wire
traffic in ~7 s, about 20 MB/s -- roughly 18% of even the mesh ceiling. Nowhere
near the cap, so the cap was not what bound it. The small-file work (4.15, 4.25,
4.26) stands.

**Large-file and link-ceiling conclusions do not survive unqualified.** They
were measured at or near a ceiling that turns out to be the wrong one.

**The 5.3 ms in-session round trip is now suspect.** 4.15 measured it, 4.50
blamed the Mac's USB adapter, and `MAX_PIPELINED_FRAMES = 2048` was tuned to
it. The switched path's ICMP RTT is **1.005 ms**. Whatever produced 5.3 ms, a
dongle on a switched segment is not obviously the whole story, and the window
knee derived from it should be re-derived rather than trusted. It is now
overridable at run time via `XSYNC_PIPELINE_FRAMES` (see `tuning.rs`), so this
costs a sweep rather than a rebuild.

## The trap: mars answers on two interfaces

`mars.local` resolves via mDNS to **both** addresses:

| interface | address | |
|---|---|---|
| `enp9s0` | 192.168.1.120 | ethernet, 1000 Mb/s |
| `wlp10s0` | 192.168.1.231 | WiFi |

Nothing warns you which one a given connection took, and a run that silently
lands on WiFi produces a plausible-looking number that is simply wrong.
**Every benchmark must prove its path**, not assume it. The harness checks
`$SSH_CONNECTION` and aborts on anything that is not the ethernet interface or
address:

```sh
conn=$(ssh "$HOST" 'echo $SSH_CONNECTION')
case "$conn" in
  *wlp10s0*|*192.168.1.231*) echo "ABORT: session took WiFi"; exit 91 ;;
  *enp9s0*|*192.168.1.120*)  : ;;
  *) echo "ABORT: cannot prove ethernet path"; exit 91 ;;
esac
```

A related nuisance: `known_hosts` holds three **stale** entries for
192.168.1.120 (ed25519/RSA/ECDSA) belonging to a previous DHCP occupant of that
address, so connecting by IP raises a host-key warning. mars's real key is
`SHA256:O2WmT1VoAuw/imCS4LZM+iKFqOEeKtZOSYhxmRWQgvk`, and it is identical
whether reached as `mars.local` or `192.168.1.120` -- the machine is fine, the
stored entries are not. Clear them with
`ssh-keygen -R 192.168.1.120` when convenient.

## Consequences for the 10 GbE work

The X540-T2 will be a **direct** Mac-to-mars connection, bypassing even the
switch. That makes it the only path in the estate with no shared
infrastructure at all, and the only one where a >1 GbE ceiling is reachable.

It follows that **the X540 cannot improve any freya or orion route**, and that
comparisons against those hosts after the upgrade will be measuring two
different networks. Baselines for the 10 GbE work belong on the Mac↔mars path
and nowhere else.

Because the onboard 1 GbE stays connected alongside the X540, the link can be
A/B'd with host, OS, build, and corpus all held constant -- which is a stronger
experiment than the estate has supported until now.
