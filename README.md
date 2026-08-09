# Windows Audio Switcher

A Windows tray app that sets any audio device as the default for Console,
Multimedia, and Communications all at once - so Discord, games, and the rest of
Windows follow the same device instead of splitting across outputs.

Left or right click the tray icon, pick a device, done. Installed copies auto-update
from GitHub Releases.

## Tech-stack
Tauri. Was originally built to be a SvelteKit/Tauri project - but the WebView is unnecessary bloat. Was stripped to just be a taskbar tray icon + context menu. All that's needed and uses much less RAM (and storage). App is ~11.8 MB downloaded, and only requires ~1.9 MB of memory.

## Why was it built?

I frequently switch between wireless earbuds, headphones and speakers as my output device - and Windows' native audio switcher only switches the default device, not the default communication device. This leads to an issue where Discord doesn't switch, leading to changing the device on Windows, then navigating into Discord's settings and switching that as well. It can also cause problems for games that have voice chat capabilities, and is very annoying to constantly workaround.

## Credits

App icon by [upnow-graphic](https://www.flaticon.com/authors/upnow-graphic) on Flaticon.
