
let templ = `
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
`;

let vars = #{
  a: #{ alias: "isong"},
  b: #{ alias: "song"},
  fmt: #{ alias: "format"},
  sub_title: #{ color: 0xff8040 },
  title_and_composer: #{ func: title_and_composer},
  count: #{ color: 0x808080 },
};

set_vars(vars);

let keys = #{
};

template(templ);


fn on_new_song(meta) {

  // Update
  log(meta.title.to_string());
  let title = if "game" in meta && meta.game != "" { 
    meta.game.to_string()
  } else { 
    meta.title.to_string()
  };
  let composer = if meta.composer == "" { "" } else { ` / ${meta.composer}` };
  if title == "" { title = meta.file_name };
  set_meta("title_and_composer", title + composer);

}

fn on_draw_screen() {

}


fn title_and_composer(meta) {
  log("START");
  log(meta.title.to_string());
  let title = if "game" in meta && meta.game != "" { 
    meta.game.to_string()
  } else { 
    meta.title.to_string()
  };
  let composer = if meta.composer == "" { "" } else { ` / ${meta.composer}` };
  if title == "" { title = meta.file_name };
  return title + composer;
}


fn message(meta) {

}
