Output assets/demo.gif

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
Type "export PEANUTBUTTER_PATH=$PWD/assets/hero-snippets:$HOME/.config/peanutbutter/snippets" Enter
Type "export XDG_CONFIG_HOME=/tmp/pb-vhs-hero-config" Enter
Type "export XDG_STATE_HOME=/tmp/pb-vhs-hero-state" Enter
Type "rm -rf /tmp/pb-vhs-hero-config /tmp/pb-vhs-hero-state" Enter
Type "mkdir -p /tmp/pb-vhs-hero-config/peanutbutter/snippets" Enter
Type "./target/debug/peanutbutter bash C-b > /tmp/pb-vhs-hero-bash" Enter
Type "source /tmp/pb-vhs-hero-bash" Enter
Type "clear" Enter
Show

Type "# Ctrl+b opens peanutbutter inline" Enter
Sleep 500ms
Ctrl+b
Sleep 1200ms

# A broad fuzzy search shows nearby matches from the normal snippet library.
Type "docker"
Sleep 1800ms
Up
Sleep 700ms
Backspace 6
Sleep 500ms

# Swap to tag mode
Ctrl+t
Sleep 1000ms
Ctrl+t 
Sleep 1000ms

# Search for log
Up
Sleep 300ms
Up
Sleep 150ms
Type "l"
Sleep 700ms
Type "ogs"
Sleep 700ms
Enter
Sleep 900ms
Up
Sleep 500ms
Enter
Sleep 1000ms

# Modify option
Down
Sleep 500ms
Down 
Sleep 500ms
Down
Sleep 300ms
Up
Sleep 700ms
Enter
Sleep 700ms

# Add comment
Type " # <- we can now run this command as we see fit"
Sleep 1000ms
