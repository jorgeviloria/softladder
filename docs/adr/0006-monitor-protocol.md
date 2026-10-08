# ADR-0006 — Online monitor and dashboard protocol

- **Status:** proposed
- **Date:** 2025
- **Deciders:** project owner

## Context

ClassicLadder's monitor is a proprietary binary protocol over UDP, serial or a PSTN/GSM modem. It
exposes: read rungs activity, read/write free and boolean variables, run/stop/reset, set target
clock, read target info, transfer a whole project, and remote network/monitor configuration — all
**without authentication or encryption**. It is also the mechanism used to deploy projects to
embedded targets.

SoftLadder needs remote observability that is safe to expose on a factory network and usable from a
browser, while remaining cheap enough for an embedded target.

## Decision

Define a new protocol with two transports:

- **JSON (default) / CBOR (optional)** frames over **TCP**, length-prefixed, plus a **WebSocket**
  variant for browsers.
- Optional **TLS** (`rustls`), **token authentication** with roles `read-only` and `read-write`, and
  per-connection audit logging of every write/force.
- Message set: `GetStatus`, `GetVars`, `SetVar`, `ForceVar`, `ReleaseVar`, `GetAlarms`,
  `AckAlarm`, `GetScanStats`, `GetTrend`, `PushProject`, `PullProject`, `SetState`, `Reset`.
- The **dashboard** is a small static web app served by `softladder-monitor` (M6) that speaks the
  same protocol over WebSocket: watch, force, trend and alarm acknowledge from any browser.
- The runtime enforces policy, not the transport: forcing requires a configurable interlock
  (maintenance mode, or PLC stopped) and always produces an audit event.

Legacy compatibility (speaking ClassicLadder's own monitor protocol so existing tooling or targets
keep working) is **deferred**; if it is implemented it will be a separate, disabled-by-default
adapter with its own ADR, because it cannot be authenticated.

## Consequences

- Remotely observable controllers without shipping a modem driver, and the same protocol serves the
  desktop UI's remote mode, the CLI and the web dashboard.
- Exposing the port becomes a conscious, documented decision with auth/TLS available by default.
- Frames are human-inspectable (JSON) which makes debugging and third-party tooling trivial; CBOR
  covers bandwidth-constrained links (Modbus/serial tunnels) when needed.
- Embedded targets need a TLS-capable stack for secure operation; plain TCP remains available for
  isolated networks, but the default configuration binds to `127.0.0.1` unless configured otherwise.

## Open questions

- Whether to expose a read-only HTTP REST surface alongside WebSocket for simple integrations.
- Retention policy for the trend buffer (ring size, sample decimation) on memory-constrained targets.
