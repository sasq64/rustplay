+++
title = ""
template = "page.html"
+++

# Terminal music player for retro formats

A text-mode music player for esoteric (8/16-bit computers and consoles, etc.) and contemporary music formats. Runs on macOS and Linux.</p>

<video controls playsinline width="100%" poster="oldplay-poster.png" src="oldplay-web.mp4"></video>

## Features

  * Plays most classic/retro music formats
  * Scans files and directories passed on the command line
  * Built-in search — indexes MODLAND (400k songs) in seconds
  * Rainbow textmode music bars (FFT visualizer)
  * Configurable layout and key bindings via Lua

## Changelog

### 0.4.0

* File system browser 
* Favorites support
* FFT improvements (better sync, visual tweaks)
* Per directory index caching (avoid re-indexing)
* FLAC format added
* Configurable key bindings and visuals using LUA

 ## Install
  You'll need Rust and cargo — get them at [https://rustup.rs](https://rustup.rs)
  ### Latest release
  {{ copy(text="cargo install oldplay") }}

  ### Development version
  {{ copy(text="cargo install --git https://github.com/sasq64/rustplay --branch dev") }}

##  Usage
  Point oldplay at a file or directory:

  `oldplay ~/music`

  See the [documentation](/doc/) for keys, configuration and search syntax.

