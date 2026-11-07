# Android 10 / low-memory frontend optimizations

This update keeps the LibreFi Rust backend, firmware, API routes, database logic, admin pages, and user-facing controls. The changes in this package are limited to the React frontend, local font assets, and frontend dependency lockfiles.

## Changes

- Timer digits are plain text (no NumberFlow digit animation).
- The timer countdown updates its own local component state once per second; it does not call the parent User state setter.
- Timer is memoized so status-only parent updates do not redraw it when its user/session prop is unchanged.
- Added a memoized `UserControls` boundary. Its `user`, refresh, play/pause, and voucher-update props remain stable during status-only updates, so the coin/voucher/rates/more controls are skipped when the page status changes but the session data does not.
- Rates are fetched only the first time the Rates drawer is opened; successful results are cached for the current page session. Failed requests can be retried by closing and reopening the drawer.
- Font loading uses local, WOFF2-subsetted Plus Jakarta Sans variable fonts. The Latin subset covers U+0000–U+00FF; the second subset retains only the Philippine peso sign (U+20B1). The variable weight axis is narrowed from 200–800 to 400–800, matching the weights used by the UI. The special brand character `ꝍ` is not present in the original Plus Jakarta Sans font, so it continues to use the browser's fallback font as it did previously.
- Removed the now-unneeded `@fontsource-variable/plus-jakarta-sans` package dependency; the small local font files are included in `src/assets/fonts/`.

## Rebuild before deployment

The checked-in `dist/` directory is retained from the supplied project. It has not been regenerated in this environment. The Rust server embeds `dist/`, so rebuild the React app before compiling/deploying the Rust binary.

On your Windows laptop, open a terminal in `librefi-reactjs` and run:

```powershell
npm ci
npm run build
```

Then return to the project root and build LibreFi using the same Rust/OpenWrt build procedure you already use. The `npm run build` command updates the root `dist/` directory through the project's Vite configuration.

## Validation

- Source-level review confirms the memoized controls receive stable props during status-only state changes.
- Font files were subsetted and checked: the Latin subset is 16,304 bytes and the peso-symbol subset is 868 bytes (17,172 bytes total), versus approximately 49,076 bytes for the prior Latin and Latin Extended files. The Vietnamese subset is no longer referenced by the source CSS.
- `tsc -b` and the Vite production build could not be run here because this source-only archive intentionally has no `node_modules`; the compiler reports missing `vite/client` and `node` type definitions. Run `npm ci` and `npm run build` on Windows to perform the full build check.
- No Rust source, firmware source, Cargo manifest/lockfile, API route, or database file was intentionally changed.
- Physical performance on an Android 10 / 2 GB device has not been measured; these changes reduce avoidable frontend work and font payload but are not a device benchmark.
