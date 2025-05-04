
let templ = `
 ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$>━━━━━━━┓
 ┃ $title_and_composer                             $>       ┃
 ┃ $sub_title                                      $>       ┃
 ┣━━━━━━━━━━━━━━━━━━┳━━━━━━┳━━━━━━━┳━━━━━━━━┳━━━━━━$>━━━━━━━┫
 ┃ $time    / $len  ┃ SONG ┃ $a/$b ┃ FORMAT ┃ $fmt $> $count┃
 ┗━━━━━━━━━━━━━━━━━━┻━━━━━━┻━━━━━━━┻━━━━━━━━┻━━━━━━$>━━━━━━━┛
  NEXT: $next_song                      $xtitle
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
  xtitle: #{ func: title_and_composer},
  count: #{ color: 0x808080 },
  sub_title: #{ color: 0xa0a0a0 },
};

set_vars(vars);

let keys = #{
};

template(templ);



fn title_and_composer(meta) {
  if "title" in meta && "composer" in meta {
    return `${meta.title} / ${meta.composer}`;
  }
  `${meta.file_name}`
}


fn message(meta) {

}
