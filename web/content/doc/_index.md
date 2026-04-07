+++
title = "Documentation"
template = "page.html"
+++

Oldplay is a terminal-based music player for retro and chiptune audio formats.

- **Commodore 64**: SID (with STIL and songlengths)
- **Amiga**: TFMX, Future Composer, AHX and many more (using UADE).
- **Trackers**: MOD, S3M, XM, IT etc
- **Consoles**: Game Boy, NES, SNES, Playstation, N64, etc
- **Modern**: Flac, MP3

## Basic Usage

```
oldplay [OPTIONS] [SONGS / DIRECTORIES...]
```

Pass one or more paths to music files or directories. Oldplay recursively scans directories for supported formats and builds a searchable index in the background.

### Options

| Flag | Description |
|------|-------------|
| `--write-config` | Write the default `config.lua` to `~/.config/oldplay/config.lua` |
| `--no-term` | Run without terminal output (headless mode) |
| `-c`, `--no-color` | Disable colored output |

### Key Bindings

The default key bindings (configurable via Lua, see [Configuration](#configuration)):

| Key | Action |
|-----|--------|
| Any letter | Start searching (enters search mode and types the letter) |
| `Space` | Play / Pause |
| `Left` / `Right` | Previous / Next subtune (for multi-song files like SID) |
| `[` / `]` | Previous / Next song in playlist |
| `0`-`9` | Jump to subtune number |
| `Up` / `Down` | Show current song list and navigate |
| `Page Up` / `Page Down` | Navigate song list by page |
| `Enter` | Play selected song or enter directory |
| `Esc` | Return to main screen |
| `=` | Add currently playing song to favorites |
| `-` / `Ctrl`+`F` | Show favorites |
| `/` | Show file/directory browser |
| `/` / `Backspace` | Go to parent directory (in directory browser) |
| `Ctrl`+`C` | Quit |

## Search

### How to Search

1. From the main screen, just type your query and press `Enter`
2. Press `Enter` to play a song, or `Esc` to go back

### Query Syntax

Multiple search terms use AND logic by default, so `purple motion` matches songs where both "purple" and "motion" appear in the title or composer fields.

The full Tantivy query syntax is supported:

| Syntax | Example | Description |
|--------|---------|-------------|
| Simple terms | `megaman` | Match songs containing "ocean" |
| Multiple terms | `rob hubbard` | AND by default -- both terms must match |
| Field-specific | `title:stardust` | Search only the title field |
| Field-specific | `composer:hubbard` | Search only the composer field |
| Phrases | `"last ninja"` | Match the exact phrase |
| Boolean | `hubbard OR galway` | OR logic between terms |
| Negation | `NOT remix` | Exclude matches |

## Configuration

Oldplay is configured through a Lua. The config controls the screen layout, key bindings, variable display, and visualizer settings.

The config is loaded from `~/.config/oldplay/config.lua`. If this file does not exist, the built-in default is used.

To generate the default config file:

```bash
oldplay --write-config
```

### Config Structure

The config script must return a Lua table with these fields:

```lua
return {
  template = "...",      -- Screen layout template string
  vars = { ... },        -- Variable display settings
  keys = { ... },        -- Key bindings
  settings = { ... },    -- Application settings (FFT, etc.)
}
```

### Template

The `template` field defines the screen layout using box-drawing characters and variable placeholders.

```
 ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$>━┳━━━━━━┓
 ┃ $title_and_composer                             $> ┃SIZE: ┃
 ┃ $sub_title                                      $> ┃$hs   ┃
 ┣━━━━━━━━━━━━━━━━━━┳━━━━━━┳━━━━━━━┳━━━━━━━━┳━━━━━━$>━┻━━━━━━┫
 ┃ $time    / $len  ┃ SONG ┃ $a/$b ┃ FORMAT ┃ $fmt $>  $count┃
 ┗━━━━━━━━━━━━━━━━━━┻━━━━━━┻━━━━━━━┻━━━━━━━━┻━━━━━━$>━━━━━━━━┛
  NEXT: $next_song

$search

 $fft
```

#### Template Syntax

| Pattern | Description |
|---------|-------------|
| `$name` | Insert the value of a variable |
| `$>char` | Fill remaining space on the line with `char` (e.g. `$>━`) |
| `$^` | Mark a line as vertically resizable |
| `$fft` | Position of the FFT visualizer |
| `$search` | Position of the search input field |

#### Built-in Variables

| Variable | Description |
|----------|-------------|
| `$title` | Song title |
| `$composer` | Composer / artist |
| `$game` | Game name (if available) |
| `$format` | Audio format name |
| `$time` | Current playback position |
| `$len` | Song length |
| `$isong` | Current subtune number |
| `$songs` | Total number of subtunes |
| `$next_song` | Name of the next song in the playlist |
| `$file_name` | Current file name |
| `$size` | File size in bytes |

### Variables (vars)

The `vars` table customizes how variables are displayed:

```lua
vars = {
  sub_title = { color = 0xff8040 },                   -- Set color (RGB hex)
  a = { alias_for = "isong" },                         -- Alias to another variable
  title_and_composer = { func = title_and_composer },  -- Compute via Lua function
}
```

| Attribute | Description |
|-----------|-------------|
| `color` | RGB color as hex integer (e.g. `0xff8040` for orange) |
| `alias_for` | Display the value of another variable |
| `func` | Lua function receiving the metadata table, returns a string |

Custom `func` functions receive a metadata table with all current song variables as fields:

```lua
local function human_size(meta)
  local size = meta.size
  if not size or size == 0 then return "?" end
  if size < 1024 then return string.format("%dB", size) end
  return string.format("%.0fKb", size / 1024)
end
```

### Key Bindings (keys)

Key bindings are defined as a list of `{ mode, key, action }` tuples:

```lua
keys = {
  { "n", "Space",  play_pause },
  { "n", "Left",   prev_subtune },
  { "n", "Right",  next_subtune },
  { "a", "ctrl-c", quit },
  { "n", ":letter:", function(x)
    focus_search()
    add_char(x)
  end },
}
```

#### Modes

| Mode | Description |
|------|-------------|
| `"n"` | Normal (main screen) |
| `"i"` | Search input |
| `"s"` | Search results screen |
| `"r"` | Result screen (search/favorites/directory) |
| `"d"` | Directory browser |
| `"f"` | Favorites screen |
| `"a"` | All modes |

Modes can be combined: `"ni"` matches both Normal and Search Input.

#### Key Names

- Letters: `"a"` through `"z"`
- Special: `"Space"`, `"Enter"`, `"Esc"`, `"BackSpace"`, `"Left"`, `"Right"`, `"Up"`, `"Down"`, `"PageUp"`, `"PageDown"`
- Modifiers: `"ctrl-c"`, `"ctrl-f"`, `"alt-x"`
- Multiple keys: `"Left,Right"` (comma-separated, same action for both)
- Patterns: `:letter:` (any letter), `:digit:` (any digit 0-9)

#### Available Actions

| Function | Description |
|----------|-------------|
| `play_pause()` | Toggle playback |
| `next_song()` | Next song in playlist |
| `prev_song()` | Previous song in playlist |
| `next_subtune()` | Next subtune in current file |
| `prev_subtune()` | Previous subtune |
| `sub_song(n)` | Jump to subtune number `n` |
| `focus_search()` | Enter search input mode |
| `add_char(c)` | Add character to search field |
| `show_favorites()` | Show favorites screen |
| `show_directory()` | Show directory browser |
| `show_main()` | Return to main screen |
| `show_current()` | Show current song list |
| `enter_or_play_selected()` | Play selected song or enter directory |
| `add_favorite(song)` | Add a song to favorites |
| `get_playing_song()` | Get the currently playing song |
| `get_selected_song()` | Get the currently highlighted song |
| `goto_parent()` | Navigate to parent directory |
| `quit()` | Exit the application |

### FFT Visualizer Settings

```lua
settings = {
  fft = {
    min_freq = 40,            -- Minimum frequency in Hz
    max_freq = 12000,         -- Maximum frequency in Hz
    visualizer_height = 5,    -- Height in terminal rows
    bar_count = 25,           -- Number of frequency bars
    bar_width = 2,            -- Width of each bar in characters
    bar_gap = 1,              -- Gap between bars in characters
    colors = { 0xf00040, 0x00ff40 }  -- Gradient colors (bottom to top)
  }
}
```

The `colors` array defines a gradient interpolated across the bar height. The default goes from red/magenta at the bottom to green at the top.

### Metadata Sidecar Files

Oldplay reads `.meta` files in TOML format to override or supplement song metadata. They are mainly used when adding favorites.

```
mysong.sid       -- the music file
mysong.sid.meta  -- the metadata
```

Example `.meta` file:

```toml
title = "Hiscore"
composer = "Rob Hubbard"
game = "Commando"
```

Any key-value pairs in the `.meta` file are merged into the song's metadata and become available as template variables.
