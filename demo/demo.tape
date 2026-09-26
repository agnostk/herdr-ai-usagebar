# VHS tape for docs/media/demo.gif. Run demo/record.sh rather than this file
# directly: it builds the plugin and starts the sandbox herdr the tape attaches to.

Output docs/media/demo.gif

Set Shell "bash"
Set FontSize 15
Set Width 1440
Set Height 780
Set Padding 10
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms
Set WindowBar Colorful
Set BorderRadius 10
Set Margin 18
Set MarginFill "#11111b"

Hide
Type "demo/sandbox.sh exec herdr"
Enter
Sleep 3s
Show

Sleep 2s
Type "# add the AI usage rows to the herdr sidebar"
Enter
Sleep 500ms
Type "herdr plugin action invoke agnostk.ai-usagebar.setup-sidebar > /dev/null"
Sleep 300ms
Enter
Sleep 4s

Type "# agents keep burning quota; pull fresh numbers"
Enter
Sleep 500ms
Type "herdr plugin action invoke agnostk.ai-usagebar.refresh > /dev/null"
Sleep 300ms
Enter
Sleep 3s
Screenshot docs/media/sidebar.png
Sleep 3s
