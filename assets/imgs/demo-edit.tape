Output assets/imgs/demo-edit.gif

Require ./target/debug/peanutbutter
Require nvim

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
Type "export PEANUTBUTTER_PATH=/tmp/pb-vhs-edit-snippets" Enter
Type "export XDG_CONFIG_HOME=/tmp/pb-vhs-edit-config" Enter
Type "export XDG_STATE_HOME=/tmp/pb-vhs-edit-state" Enter
Type "rm -rf /tmp/pb-vhs-edit-config /tmp/pb-vhs-edit-state /tmp/pb-vhs-edit-snippets" Enter
Type "mkdir -p /tmp/pb-vhs-edit-config/peanutbutter/snippets /tmp/pb-vhs-edit-snippets" Enter
Type "cp assets/demo-snippets/snippets.md /tmp/pb-vhs-edit-snippets/snippets.md" Enter
Type "export VISUAL=nvim" Enter
Type "./target/debug/peanutbutter bash C-b > /tmp/pb-vhs-edit-bash" Enter
Type "source /tmp/pb-vhs-edit-bash" Enter
Type "clear" Enter
Show

Type "# Ctrl+E edits the selected snippet, then reloads" Enter
Sleep 500ms
Ctrl+b
Sleep 900ms

Type "docker shell"
Sleep 800ms
Ctrl+e
Sleep 1200ms

Type "oEdited from the picker with Ctrl+E."
Sleep 900ms
Escape
Sleep 300ms
Type ":"
Sleep 250ms
Type "wq"
Sleep 700ms
Enter
Sleep 1500ms

Enter
Sleep 900ms
Type " # edited snippet reloaded into the picker"
Sleep 1600ms
