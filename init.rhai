
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
`)
