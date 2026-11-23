# Regenerate the CLI demo GIF in the README.
#
#   brew install vhs
#   vhs scripts/demo.tape
#
# The recording uses the vault `test` so it never touches real secrets.

Output docs/images/seal-cli.gif

Set Shell "zsh"
Set FontFamily "Fira Code"
Set FontSize 17
Set LineHeight 1.5
Set Padding 24
Set Width 760
Set Height 380
Set Framerate 30
Set TypingSpeed 40ms
Set CursorBlink true
Set Theme { "Background": "#0d0d0d", "Foreground": "#ededed", "Cursor": "#3b82f6", "Selection": "#22355c", "Black": "#1e2430", "Red": "#f87171", "Green": "#4ade80", "Yellow": "#f38020", "Blue": "#60a5fa", "Purple": "#c084fc", "Cyan": "#22d3ee", "White": "#ededed", "BrightBlack": "#8a8a8a", "BrightRed": "#fca5a5", "BrightGreen": "#86efac", "BrightYellow": "#fdba74", "BrightBlue": "#93c5fd", "BrightPurple": "#d8b4fe", "BrightCyan": "#67e8f9", "BrightWhite": "#ffffff" }

Hide
Type "export PS1='$ '"
Enter
Sleep 500ms
Type "clear"
Enter
Show

Sleep 800ms
Type "seal set test/secret-foo bar"
Enter
Sleep 700ms

Type "seal set test/secret2 bar2"
Enter
Sleep 700ms

Type "seal get test/secret-foo"
Enter
Sleep 700ms

Type "seal list 'test/*'"
Enter
Sleep 2500ms
