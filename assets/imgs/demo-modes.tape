Output assets/imgs/demo-modes.gif

Require ./target/debug/peanutbutter

Set Shell "bash"
Set FontSize 24
Set Width 1400
Set Height 760
Set WindowBar Colorful
Set WindowBarSize 36
Set Padding 16
Set Framerate 24
Set TypingSpeed 70ms

Hide
Type "export PATH=$PWD/target/debug:$PATH" Enter
Type "export PEANUTBUTTER_PATH=$PWD/assets/demo-snippets" Enter
Type "export XDG_CONFIG_HOME=/tmp/pb-vhs-modes-config" Enter
Type "export XDG_STATE_HOME=/tmp/pb-vhs-modes-state" Enter
Type "rm -rf /tmp/pb-vhs-modes-config /tmp/pb-vhs-modes-state" Enter
Type "mkdir -p /tmp/pb-vhs-modes-config/peanutbutter/snippets" Enter
Type "./target/debug/peanutbutter bash C-b > /tmp/pb-vhs-modes-bash" Enter
Type "source /tmp/pb-vhs-modes-bash" Enter
Type "clear" Enter
Show

Type "# Search highlights names, paths, tags, and snippet bodies" Enter
Sleep 500ms
Ctrl+b
Sleep 1000ms

Type "docker"
Sleep 1700ms

Backspace 6
Sleep 400ms

Type "tag:docker"
Sleep 1400ms

Type " body:logs"
Sleep 1800ms

Backspace 21
Sleep 500ms

Type "logs"
Sleep 1500ms

Backspace 4
Sleep 500ms

Ctrl+t
Sleep 900ms
Type "ops"
Sleep 900ms
Backspace 3
Sleep 400ms

Ctrl+t
Sleep 900ms
Type "logs"
Sleep 700ms
Enter
Sleep 900ms
Down
Sleep 500ms
Enter
Sleep 900ms
Escape
Sleep 700ms
