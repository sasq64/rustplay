
function title_and_composer(meta)
  log("START")
  log(tostring(meta.title or ""))
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
 ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$>━━━━━━━┓
 ┃ $title_and_composer                             $>       ┃
 ┃ $sub_title                                      $>       ┃
 ┣━━━━━━━━━━━━━━━━━━┳━━━━━━┳━━━━━━━┳━━━━━━━━┳━━━━━━$>━━━━━━━┫
 ┃ $time    / $len  ┃ SONG ┃ $a/$b ┃ FORMAT ┃ $fmt $> $count┃
 ┗━━━━━━━━━━━━━━━━━━┻━━━━━━┻━━━━━━━┻━━━━━━━━┻━━━━━━$>━━━━━━━┛
  NEXT: $next_song
@isong=a
@songs=b
@format=fmt
@count=:#808080
@sub_title=:#a0a0a0
@title_and_composer=:#ffffff
@TEXT=:#20e020
]]

local vars = {
  a = { alias = "isong" },
  b = { alias = "song" },
  fmt = { alias = "format" },
  sub_title = { color = 0xff8040 },
  title_and_composer = { func = title_and_composer },
  count = { color = 0x808080 },
}

local keys = {
  { "[", next_song }
}


set_vars(vars)
template(templ)
