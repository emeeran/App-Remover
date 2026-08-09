# App Remover — User Manual

**App Remover** is a Linux desktop application that lets you safely remove
applications *and* the files they leave behind ("residue") across every common
package source on your system — apt/dpkg, snap, flatpak, pip, npm, systemd user
units, and AppImages. Before anything is deleted it shows you exactly what will
happen, backs up what it can restore, records an audit trail, and can undo a
removal afterwards.

> App Remover never removes anything without an explicit **Approve** and
> **Begin removal** from you. Protected system packages cannot be removed at
> all.

---

## 1. Requirements

- A Debian/Ubuntu-based Linux distribution (`.deb` package).
- Root rights for removing **system** packages (apt/snap/flatpak) — App Remover
  raises a **polkit** password prompt when needed.
- The runtime libraries the package depends on (installed automatically):
  `libwebkit2gtk-4.1-0`, `libgtk-3-0`.

## 2. Installation

```bash
sudo dpkg -i "App Remover_0.1.0_amd64.deb"
# if dependencies are missing:
sudo apt-get install -f
```

Then launch it from your application menu (search "App Remover") or run:

```bash
app-remover
```

The binary installs to `/usr/bin/app-remover`.

## 3. The window at a glance

- **App bar (top):** the App Remover logo and name, with buttons for the
  **🌙/☀ theme toggle**, **About**, **History**, and **Refresh**.
- **Meta line:** a quick status readout — database health, total / removable /
  protected counts, and how many package sources were detected (e.g. `6/8 sources`).
- **Left panel — Inventory:** every detected application with a filter box. Each
  row shows the name with a `version · source` subtitle and a status:
  - **removable** — can be processed.
  - **🔒 protected** — on the non-overridable blocklist; cannot be removed.
- **Right panel — Detail:** the residue scan, removal plan, and job for the app
  you clicked, headed by a **workflow stepper** (Scan → Plan → Approve → Remove)
  that tracks your progress through the flow.
- **About** and **History** open as modal dialogs (close with ✕, the backdrop,
  or the **Esc** key).

---

## 4. Core concepts (what you're looking at)

| Term | Meaning |
|---|---|
| **Residue** | Files and directories associated with an app — binaries, config, cache, data, services, desktop entries, etc. |
| **Owner set / usage** | Whether a residue item belongs to this app alone (**exclusive**), to several apps (**shared**), or to the OS (**system**). Only *exclusive* items are ever deleted. |
| **Verdict** | A plan operation's safety: **safe**, **risky** (other packages depend on it — you must accept the risk), **blocked** (cannot proceed). |
| **Snapshot** | A pre-removal backup (files copied + the package recorded for reinstall) that enables undo. |
| **Audit record** | An append-only, tamper-evident entry written when a job finishes, recording exactly what was planned and what happened. |

---

## 5. Removing an application (the workflow)

1. **Find the app.** Click a row in the Inventory, or type in the filter box to
   narrow the list. Protected apps are dimmed and not clickable.
2. **Scan.** Clicking an app runs a **residue scan** automatically. The detail
   panel shows:
   - counts of **exclusive / shared / system** items,
   - any **runtime warnings** (e.g. "3 running processes match", "service is
     active"),
   - the residue **grouped by category** (binary, config, cache, data, …).
3. **Compose a plan.** Click **Compose removal plan**. App Remover builds a
   dry-run plan: the package uninstall plus deletion of leftover files, each
   with a verdict:
   - **safe** — proceed normally.
   - **risky** — another package depends on this one; tick **accept** to allow it.
   - **blocked** — listed under *blocked reasons*; the plan can't be approved
     until the blocker is resolved.
4. **Approve.** Accept any risks you're comfortable with, then click
   **Approve plan**. The status changes to **approved**.
5. **Create the job.** Click **Create job (capture snapshot)**. This backs up the
   removable residue and records the package for reinstallation. Nothing is
   deleted yet.
6. **Begin removal.** Click **▶ Begin removal**. A polkit prompt asks for your
   password (for system packages); the operations then run in order. The panel
   shows each step's status and the final outcome:
   - **completed** — the app (and accepted residue) is gone.
   - **rolled back** — a step failed, so App Remover automatically restored
     everything from the snapshot.
   - **failed** — a step failed *and* rollback could not complete (rare; see
     Troubleshooting).

> Tip: To try the flow safely, install a throwaway package first
> (`sudo apt install -y ed`) and remove that.

---

## 6. Undoing a removal

Click **History** (top right) to see recent completed jobs. For any **undoable**
entry, click **Undo**. App Remover verifies the snapshot's integrity, reinstalls
the package, and restores the backed-up files. Packages that could not be
reinstalled are listed as *deferred*.

---

## 7. Safety model

- **Protected applications.** A core set is non-overridable and can never be
  removed, including: `app-remover`, `gnome-shell`, `gnome-session`,
  `kde-plasma`, `xfce4-session`, `glibc`, `libc6`, `libstdc++6`, `xorg`,
  `xwayland`, `mutter`, `kwin`, `systemd`, `dpkg`, `apt`, `snapd`, `flatpak`.
- **Snapshots + auto-rollback.** Every removal captures a snapshot first. If any
  step fails, the job is automatically rolled back from that snapshot.
- **Least privilege.** Removal runs through `pkexec`/polkit per operation (no
  persistent root shell), using argument arrays (no shell). apt package names
  are validated against a strict allowlist when scanned; extending that
  execution-boundary validation to every backend is tracked in `AUDIT.md`.
- **Tamper-evident audit.** Each finished job appends a hash-chained record; the
  chain detects after-the-fact tampering.

## 8. Configuration (optional)

App Remover reads an optional JSON config at:

```
~/.config/app-remover/config.json
```

```json
{
  "blocklist_user": ["my-important-tool"],
  "snapshot_cost_cap_bytes": 5368709120
}
```

- `blocklist_user` — extra package names to treat as protected (appended to the
  core blocklist).
- `snapshot_cost_cap_bytes` — if a removal's snapshot would exceed this size
  (default **1 GiB**), the job refuses to start unless you opt in.

If the file is missing or invalid, defaults are used.

## 9. Supported backends

| Backend | Detected via | Removal command |
|---|---|---|
| apt / dpkg | `/usr/bin/dpkg` | `apt-get remove` / `purge` (root) |
| snap | `/usr/bin/snap` | `snap remove` (root) |
| flatpak | `/usr/bin/flatpak` | `flatpak uninstall` (root) |
| pip | `/usr/bin/pip3` | `pip3 uninstall` (user) |
| npm | `/usr/bin/npm` | `npm uninstall -g` (user) |
| systemd | `/usr/bin/systemctl` | `systemctl --user stop` + `disable` |
| AppImage | `~/Applications`, `~/Downloads` scan | deletes the file |
| manual | always available | via residue file deletion |

## 10. Where data lives

- **Database / audit:** `~/.local/share/app-remover/app-remover.db`
- **Snapshots (file backups):** `~/.local/share/app-remover/snapshots/<id>/`
- **Config:** `~/.config/app-remover/config.json`

Snapshots are kept so undo remains possible; you may delete a snapshot directory
for a job you no longer intend to undo.

---

## 11. Troubleshooting

- **"Begin removal" did nothing / prompted then failed.** A step likely failed.
  Check the job's step list for the failed operation and its message. If the job
  shows **rolled back**, your system was restored automatically.
- **A package I expected isn't listed.** App Remover only shows backends whose
  tools are installed (see the chips in the header). pip/npm inventories can be
  large — use the filter box.
- **polkit prompt doesn't appear.** Ensure a polkit authentication agent is
  running on your desktop session (most desktop environments provide one).
- **Undo says a package was "deferred".** The package couldn't be reinstalled
  (e.g. no longer in your repositories). Your files were still restored.
- **The app won't start.** Confirm the WebKit/GTK dependencies are installed
  (`sudo apt-get install -f`).

## 12. FAQ

- **Does it need root to run?** Only to *remove system* packages. The app itself
  runs as your user; polkit asks for a password only when a root operation is
  performed.
- **Can it remove something it considers "shared"?** No — items shared with other
  apps are marked **blocked** to protect them. Remove the dependency first.
- **Is the removal reversible?** Yes, for completed jobs, via **Undo** in History
  (until you delete the snapshot).

## 13. Known limitations

- Multi-owner (shared) residue detection is simplified; some items are treated
  as exclusive when they may be shared — review the plan before approving.
- pip and npm inventories include libraries, not just end-user apps.
- Database schema changes are applied on a fresh database only (no in-place
  migration); removing `app-remover.db` resets history.
