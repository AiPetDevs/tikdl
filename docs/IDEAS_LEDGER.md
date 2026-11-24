# Ideas ledger — tried: accepted / rejected and why

| Date | Idea | Outcome | Why |
|------|------|---------|-----|
| 2026-11-23 | Hand the downloaded file over from the store to the folder (`ExportMode::TryReference`, move instead of copy) to avoid 2× disk use | REJECTED | Store keeps an `External` entry for the moved path; user deletes/renames the file → next download of the same hash fails (`Error::Io`). Entry can't be removed (delete/GC private). Waiting for GC did not help |
| 2026-11-23 | Default export (Copy) + GC every 15 s | ACCEPTED | On APFS the copy is a reflink clone: 400 MB file used 411 MB total; store back to 72 KB after GC. Other OS: space check asks for 2× during hand-over |
| 2026-11-23 | Add `site` to `ui-check.json` roots | REJECTED by gate | Gate settings change only by the owner; landing page moved under `ui/site` instead |
| 2026-11-23 | Keep IP addresses in links | REJECTED | Leaks sender IP; relay-only address still connects, direct path found after connect |
