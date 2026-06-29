# tbwatcher

A free, lightweight, **pixel-art** desktop companion for **TBH: Task Bar Hero** that helps players trade
smarter on the **Steam Community Market** — live prices, watchlist price history, a public-inventory
portfolio with alerts, and a read-only stash valuation — while the game idles in the taskbar.

> This is a public **source snapshot** of the app. It runs quietly in the system tray and is built to be
> easy on resources (small installer, low idle footprint).

## Features

- **Market Watch** — live prices and sale volume for TBH items; a personal watchlist with search.
- **Price History** — a per-item price chart, self-accumulated locally while your watchlist polls (no
  login, no full-catalog scan).
- **Portfolio & Alerts** — value and P/L of your holdings from your **public** Steam inventory (by
  SteamID, no login), plus price-threshold notifications.
- **Stash & Sell Advisor** — reads your local save **read-only** to value your full in-game tradeable
  inventory and rank what's best to list, given the game's limited Market listing slots.

Free to use, with generous usage limits; an optional subscription lifts them.

## Tech stack

[Tauri 2](https://tauri.app) (Rust core) + [SvelteKit](https://kit.svelte.dev) / TypeScript frontend.

## Develop

```sh
npm install
npm run tauri dev     # run the app
npm run check         # svelte-check
cargo test --manifest-path src-tauri/Cargo.toml   # Rust tests
```

## Build

```sh
npm run tauri build   # produces an NSIS installer + MSI under src-tauri/target/release/bundle/
```

## Compliance

Uses only Steam's own market surfaces and the player's consented, read-only local save. No trade
automation, no save editing, no game-memory access.

## Credits

In-game item → market-name data is derived from the MIT-licensed
[`shigake/tbh-copilot`](https://github.com/shigake/tbh-copilot) project, with attribution preserved in
`src-tauri/src/save/item_map.gen.cjs`.

## License

Proprietary — all rights reserved. Source is published for transparency; it is not licensed for reuse.
