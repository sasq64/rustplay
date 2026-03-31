
# TODO

### V1 TODO

- [x] Resampling (for N64)
- [x] Show file format in search
- [x] Show file name only when meta data not known
- [x] Correct color / no color rendering
- [x] Fix double name for game music
- [x] Start playing all songs
- [x] Hide song count after done indexing
- [x] Make cargo installable
- [x] Catch c++ exceptions

### NICE FEATURES

- [ ] Random play
- [x] Play/Pause
- [ ] Size meta data
- [ ] Max errors, and show filename
- [x] Favorites play list
- [ ] Goto author
- [ ] Goto directory
- [ ] song/startSong from .meta should be used
- [ ] length from meta should be used
- [ ] Letter keys in menu should jump to matching

### ISSUES

- [x] Fix no color mode
- [x] No FFT when sample data is non-power-of-two
- [x] FFT delay for bluetooth
- [ ] Song count 0/empty
- [x] Use 'zip' directly
- [x] Open Startrekker with UADE
- [x] Go to parent dir should select previous
- [x] Separate menu state for all menues
- [ ] No MP3 song length
- [ ] Color of time elapsed
- [x] Navigate with no previous should go to files
- [o] Select current playing in menu if possible
- [ ] title_and_composer <- base filename if no title
- [x] Relative dir cant go to parent

### REFACTOR

- [ ] Plugin interface on rust side

### VISUAL IMPROVEMENTS

- [ ] Pause time fade animation
- [?] Fade search selector
- [ ] File format color
- [ ] Scroll sub_title
- [ ] Fade/toggle sub title for tracker msg
- [ ] Fade in/out messages
- [ ] Add colors to template
- [ ] Parse json dur files for color data

### SCRIPTABLE LOOK

- [ ] RHAI functions as template variables

### CODE STRUCTURE

- [x] Refactor to main + lib
