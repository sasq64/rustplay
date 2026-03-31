local function human_size(meta)
  local size = meta.size
  if not size or size == 0 then return "" end
  if size < 1024 then
    return string.format("%dB", size)
  elseif size < 1024 * 1024 then
    return string.format("%.0fKb", size / 1024)
  elseif size < 1024 * 1024 * 1024 then
    return string.format("%.1fMb", size / (1024 * 1024))
  else
    return string.format("%.1fGb", size / (1024 * 1024 * 1024))
  end
end

local function title_and_composer(meta)
  local title
  if meta.game and meta.game ~= "" then
    title = tostring(meta.game)
  else
    title = tostring(meta.title or "")
  end
  local composer
  if not meta.composer or meta.composer == "" then
    composer = ""
  else
    composer = " / " .. tostring(meta.composer)
  end
  if title == "" then title = meta.file_name or "" end
  return title .. composer
end

local templ = [[
 ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$>━┳━━━━━━┓
 ┃ $title_and_composer                             $> ┃SIZE: ┃
 ┃ $sub_title                                      $> ┃$hs   ┃
 ┣━━━━━━━━━━━━━━━━━━┳━━━━━━┳━━━━━━━┳━━━━━━━━┳━━━━━━$>━┻━━━━━━┫
 ┃ $time    / $len  ┃ SONG ┃ $a/$b ┃ FORMAT ┃ $fmt $>  $count┃
 ┗━━━━━━━━━━━━━━━━━━┻━━━━━━┻━━━━━━━┻━━━━━━━━┻━━━━━━$>━━━━━━━━┛
  NEXT: $next_song

$search
 $fft
 .
 .
 .
 .
]]


local vars = {
  a = { alias_for = "isong" },
  b = { alias_for = "songs" },
  fmt = { alias_for = "format" },
  sub_title = { color = 0xff8040 },
  title_and_composer = { func = title_and_composer },
  hs = { func = human_size },
  count = { color = 0x808080 },
}
local keys
if false then
  keys = {
    { "ns", ":letter:", function(x)
      focus_search()
      add_char(x)
    end },
    { "r", "Enter", enter_or_play_selected },
    { "r", "Esc",   show_main },
    { "ni", "Up,Down,PageUp,PageDown", function(x)
      show_current()
      add_char(x)
    end },
    { "n", "a", function()
      add_favorite(get_playing_song())
    end },
    { "r", "a", function()
      add_favorite(get_selected_song())
    end },
    { "a", "ctrl-c",      quit },
    { "n", "f,+,-",       show_favorites },
    { "n", "/",           show_directory },
    { "d", "/,BackSpace", goto_parent },
    { "a", "]",           next_song },
    { "a", "[",           prev_song },
    { "n", ":digit:", function(c)
      sub_song(tonumber(c))
    end },
  }
else
  keys = {
    { "n", "s",     focus_search },
    { "n", "Left",  prev_subtune },
    { "n", "Right", next_subtune },
    { "n", "Up,Down,PageUp,PageDown", function(x)
      show_current()
      add_char(x)
    end },
    { "r", "Enter", enter_or_play_selected },
    { "r", "Esc",   show_main },
    { "n", "a", function()
      add_favorite(get_playing_song())
    end },
    { "r", "a", function()
      add_favorite(get_selected_song())
    end },
    { "a", "ctrl-c",      quit },
    { "n", "f",           show_favorites },
    { "n", "d,/",         show_directory },
    { "d", "/,BackSpace", goto_parent },
    { "n", "n",           next_song },
    { "n", "p",           prev_song },
    { "a", "ctrl-n",      next_song },
    { "a", "ctrl-p",      prev_song },
    { "n", ":digit:", function(c)
      sub_song(tonumber(c))
    end },
  }
end

return {
  template = templ,
  vars = vars,
  keys = keys,
}
