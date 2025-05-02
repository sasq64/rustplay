
template(`
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
`);


add_alias("b", "song");
add_alias("a", "isong");
add_alias("count", 0x808080);
add_alias("sub_title", 0xa0a0a0);
// add_alias("full_name", |meta| meta.title + " / " + meta.composer)

fn title_and_composer(meta) {
  if "title" in meta && "composer" in meta {
    return `${meta.title} / ${meta.composer}`;
  }
  `${meta.file_name}`
}

