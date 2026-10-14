# Plugins

Scylla reads a site only if a plugin for that site exists. A plugin is a `.wasm` file that scrapes one domain.

> [!WARNING]
> Plugins run WebAssembly on your machine with network access through the host. Read the plugin source or install only from sources you trust.

## Install

From the app: open the command palette with `:`, choose "Install Plugin from GitHub", and paste a repo URL such as `https://github.com/owner/repo`. Scylla downloads the repo's latest release, installs every `.wasm` asset it finds (the `plugin-` prefix sets the domain), and registers the plugin without a restart.

By hand: drop the file into `~/.config/scylla-reader/plugins/` with the name `plugin-<domain>.wasm`. The filename sets the domain, so `plugin-example.com.wasm` serves `example.com`.

## Config

A plugin with a config schema writes its settings to `~/.config/scylla-reader/plugin-<domain>.json`. Edit the file directly; the server picks the values up on load. The command palette (`:`) also opens Plugin Configs to edit them without leaving the app.

## Write one

The reference template, the shared API crate, and a full guide live in [scylla-plugin-base](https://github.com/MothInBox/scylla-plugin-base). A plugin exports:

- `get_config_schema`: the `ConfigField` descriptors the UI renders
- `scrape_book`: metadata plus the chapter list
- `scrape_chapter`: the text of one chapter

It reaches the network only through the `curl_fetch` host function, and calls `scylla_fail` to report errors back to the server.
