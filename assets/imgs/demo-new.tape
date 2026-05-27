Output assets/imgs/demo-new.gif

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
Type "export PEANUTBUTTER_PATH=/tmp/pb-vhs-new-snippets" Enter
Type "export XDG_CONFIG_HOME=/tmp/pb-vhs-new-config" Enter
Type "export XDG_STATE_HOME=/tmp/pb-vhs-new-state" Enter
Type "rm -rf /tmp/pb-vhs-new-config /tmp/pb-vhs-new-state /tmp/pb-vhs-new-snippets" Enter
Type "mkdir -p /tmp/pb-vhs-new-config/peanutbutter/snippets /tmp/pb-vhs-new-snippets" Enter
Type "set -o history" Enter
Type "history -s git switch feature/readme-demo" Enter
Type "history -s curl https://api.example.test/users -o /tmp/users.json" Enter
Type "./target/debug/peanutbutter bash C-b > /tmp/pb-vhs-new-bash" Enter
Type "source /tmp/pb-vhs-new-bash" Enter
Type "clear" Enter
Show

Type "# Save a recent command without leaving the terminal" Enter
Sleep 500ms
Type "pb new" Enter
Sleep 1000ms

Type "curl"
Sleep 700ms
Enter
Sleep 700ms

Type "Fetch users"
Sleep 600ms
Enter
Sleep 600ms
Space
Sleep 500ms
Down
Sleep 300ms
Space
Sleep 500ms
Enter
Sleep 1000ms

Type "pb" Enter
Sleep 900ms
Type "fetch"
Sleep 700ms
Escape
Sleep 700ms
