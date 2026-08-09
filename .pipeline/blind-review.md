# Blind Review (Phase 5)

Produced by a fresh subagent dispatched with a **sanitized prompt**: it was told
only that this is a Tauri desktop app that removes packages, and asked for an
ordinary cold due-diligence review. It was **not** told a cleanup pipeline ran,
nor about phases, `trash2review`, `AUDIT.md`, or that any refactor occurred.

**Honest limit:** this subagent runs on the same underlying model as the rest of
the work. A sanitized, context-blind pass removes *self-grading bias* and
*framing bias* (both real and both fixed by the sanitized dispatch). It does
**not** remove blind spots the model has regardless of context. For a genuinely
independent signal beyond this phase, paste a sample of the finished code into a
brand-new session with no relation to this one (fresh terminal, fresh
conversation, ideally a different reviewer) and ask the same cold-review
question. This file is not a full substitute for that.

---

Verbatim findings from the reviewer:

**CRITICAL | executor.rs:89-104 | Privilege escalation has no output sanitization**
- `run_privileged()` executes arbitrary commands via `pkexec` with user-controlled package names but doesn't sanitize stderr output before logging
- Error messages from failed privileged commands could leak system information or contain malicious input if package names are crafted

**CRITICAL | executor.rs:62-70 | File deletion lacks race condition protection**
- `delete_path()` has a classic TOCTOU vulnerability: checks if path is dir/file, then deletes it separately
- Between `is_dir()` check and `remove_dir_all()`, an attacker could symlink swap to delete arbitrary files
- No validation that resolved paths are within expected boundaries

**CRITICAL | snapshot.rs:116-137 | Snapshot capture doesn't validate source paths**
- Backs up files without verifying they're actually owned by the application being removed
- Could be exploited to copy sensitive files (ssh keys, browser data) by crafting malicious artifacts

**CRITICAL | runner.rs:209-214 | Rollback verification only checks checksums**
- `verify()` only validates file integrity, not that snapshot contents match what was captured
- A compromised snapshot could pass verification if attacker controls both source and backup

**HIGH | commands.rs:97-134 | Desktop file resolution has multiple injection points**
- `which_exec()` uses naive PATH splitting and doesn't validate resolved binaries
- `resolve_desktop()` trusts Exec= content implicitly, could execute arbitrary commands
- No validation that resolved binaries actually belong to the package they claim to

**HIGH | planner.rs:34-38 | Reverse dependency checking incomplete**
- Only checks apt reverse dependencies, ignores snap/flatpak/pnpm interdependencies
- Could break system by removing packages required by other install methods

**HIGH | backends/apt.rs:115-135 | Residue detection has unsafe filesystem access**
- `symlink_metadata()` on user-controlled paths without boundary checking
- Could be exploited to probe filesystem or trigger side-effects

**HIGH | snapshot.rs:124-139 | copy_dir recursive implementation has no depth limiting**
- No protection against directory traversal attacks or symlink loops
- Could be exploited to cause DoS or copy sensitive data

**HIGH | scanner.rs:11-44 | Scan creation has no authorization checks**
- Anyone who can invoke `create_scan` can scan any application
- Could be abused to probe system state or fingerprint installed packages

**HIGH | db.rs:22-24 | Database has no encryption or integrity validation**
- SQLite stored in plain text in XDG data directory
- No verification that database hasn't been tampered with between runs

**MEDIUM | backends/apt.rs:192-215 | Reverse dependency parsing fragile**
- Manual parsing of apt-cache output without proper escaping
- Could break with unusual package names or apt versions

**MEDIUM | residue.rs:95-110 | Leftover path checking is existence-only**
- `leftover_paths()` only checks if paths exist, not if they're actually owned by target package
- Could attribute unrelated files to application

**MEDIUM | runner.rs:99-187 | Job execution has no timeout mechanism**
- Long-running package operations could hang indefinitely
- No way to cancel in-progress jobs

**MEDIUM | audit.rs:29-85 | Hash chain validation not enforced on read**
- `load()` doesn't verify hash chain integrity
- Compromised audit records could be injected without detection

**MEDIUM | lib.rs:3 | Dead code allowed globally**
- `#![allow(dead_code)]` masks unused code that might be incomplete implementations
- Could indicate unfinished security-critical code

**LOW | commands.rs:18-26 | Health check has no rate limiting**
- Could be abused to probe system state or timing attacks
- No authentication on administrative endpoints

**LOW | policy.rs:10-28 | Protected package list is hardcoded**
- Can't be updated without recompilation
- Missing critical packages like `bash`, `coreutils`

**LOW | backends/*.rs | Backend detection has no fallback**
- If tool exists but is broken/incompatible, app still tries to use it
- Could leave system in inconsistent state

**LOW | dto.rs:1-55 | Frontend types have no validation**
- Types don't enforce constraints (lengths, formats) that backend expects
- Could crash on malformed input

**TESTING | All modules | Destructive operations have no integration tests**
- `executor.rs` priv operations only mocked, never tested against real system
- `snapshot.rs` restore path not tested on real filesystem state
- No end-to-end tests of actual package removal/rollback

**TESTING | All modules | No failure injection testing**
- No tests for disk full, network timeout, permission denied scenarios
- Rollback logic never tested under realistic failure conditions

**TESTING | backend/tests/integration/api.test.ts:1-238 | Integration tests mock everything**
- Node backend integration tests are against seeded data, not real packages
- Don't validate actual destructive operations

**ARCHITECTURE | Overall | Reference backend vs. Tauri implementation divergence**
- Node backend has comprehensive validation (Zod schemas), Rust backend has minimal validation
- Different security models between reference implementation and production code

**ARCHITECTURE | CI | No security scanning in CI pipeline**
- `.github/workflows/ci.yml` only runs lint and tests
- No static analysis, dependency scanning, or fuzz testing

Reviewer's bottom line: this is a destructive system-administration tool running
with root privileges (via pkexec) that lacks comprehensive security validation,
has insufficient testing of the destructive paths, and has several potential
injection vectors; the snapshot/rollback system is well-designed conceptually but
has implementation gaps.
