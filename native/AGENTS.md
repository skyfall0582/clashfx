# Native application delivery

These instructions supplement the repository's root `AGENTS.md`.

- After every native application update, run `scripts/build.ps1` from `native` and deliver the complete application to `G:\MY_APP_PROJECTS\ClashFx\native\releases\ClashFx`.
- Keep `releases/ClashFx/ClashFx.exe` current even when also producing an installer. A binary in `target` or an installer alone does not satisfy delivery.
- Keep the main page's outer padding at 2–4 px and navigation and layouts compact, following the product UI design skill while preserving readable text and usable controls.
