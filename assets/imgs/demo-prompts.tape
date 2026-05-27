Output assets/imgs/demo-prompts.gif

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
Type "export XDG_CONFIG_HOME=/tmp/pb-vhs-prompts-config" Enter
Type "export XDG_STATE_HOME=/tmp/pb-vhs-prompts-state" Enter
Type "rm -rf /tmp/pb-vhs-prompts-config /tmp/pb-vhs-prompts-state" Enter
Type "mkdir -p /tmp/pb-vhs-prompts-config/peanutbutter/snippets" Enter
Type "./target/debug/peanutbutter bash C-b > /tmp/pb-vhs-prompts-bash" Enter
Type "source /tmp/pb-vhs-prompts-bash" Enter
Type "clear" Enter
Show

Type "# Suggestions and dependent defaults" Enter
Sleep 500ms
Ctrl+b
Sleep 900ms

Type "dependent"
Sleep 700ms
Enter
Sleep 700ms

Down
Sleep 500ms
Tab
Sleep 800ms

Down
Sleep 500ms
Tab
Sleep 800ms

Enter
Sleep 1200ms
Type " # reviewed before running"
Sleep 1800ms
