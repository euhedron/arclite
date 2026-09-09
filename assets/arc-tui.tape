# The arc TUI's surfaces as literal screenshots — the real interactive binary driven by scripted
# keystrokes (VHS: https://github.com/charmbracelet/vhs; `brew install vhs`). Run from the repo
# root: `vhs assets/arc-tui.tape`. Regenerates the README's GIF in place and one PNG per surface
# into `tui-shots/` (gitignored — the tape is the source; the shots are its output).
# The hidden setup builds and runs the checkout's own binary, so the shots document this code,
# never whatever `arc` happens to sit on PATH; the footer's arrival gates the first frame.
# Window height: the 24-row inline viewport plus scrollback headroom.

Output assets/arc-tui.gif

Set FontSize 14
Set Width 1500
Set Height 800
Set Padding 16
Set TypingSpeed 30ms

Hide
Type "mkdir -p tui-shots && cargo build -q && clear && ./target/debug/arc tui"
Enter
Wait /commands/
Sleep 1s
Show

Screenshot tui-shots/home.png

Type "/"
Sleep 1s
Screenshot tui-shots/palette.png

Type "status"
Sleep 500ms
Enter
Sleep 3s
Screenshot tui-shots/status.png

Type "/"
Sleep 500ms
Type "config"
Sleep 500ms
Enter
Sleep 2s
Screenshot tui-shots/config.png

Type "/"
Sleep 500ms
Type "rules"
Sleep 500ms
Enter
Sleep 2s
Screenshot tui-shots/rules.png

Type "/"
Sleep 500ms
Type "items"
Sleep 500ms
Enter
Sleep 2s
Screenshot tui-shots/items.png

Type "/"
Sleep 500ms
Type "log"
Sleep 500ms
Enter
Sleep 2s
Screenshot tui-shots/log.png

Type "/"
Sleep 500ms
Type "usage"
Sleep 500ms
Enter
Sleep 2s
Screenshot tui-shots/usage.png

Type "/"
Sleep 500ms
Type "feedback"
Sleep 500ms
Enter
Sleep 2s
Screenshot tui-shots/feedback.png

Type "/"
Sleep 500ms
Type "doctor"
Sleep 500ms
Enter
Sleep 2s
Screenshot tui-shots/doctor.png

Type "q"
Sleep 1s
