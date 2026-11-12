# Testing Minimap Nudge 2.0 on your PC

The schedule logic, sound decoding and game detection were tested on Linux and as a Windows build under Wine. These checks cover the parts that need real Windows: the tray, the settings window (WebView2), actual sound output, and a real League client. They take about 10 minutes plus one game.

## 1. Before a game (5 min)

1. Run the installer (or `minimap-nudge.exe` from the zip). SmartScreen will warn you: click **More info → Run anyway**.
2. The settings window should open, and a grey radar icon should appear in the tray. If the icon is hidden, it's under the **^** arrow.
3. Click the ▶ next to each sound. You should hear all 5.
4. Click **Upload your own…** and pick an mp3 from your PC. It should play straight away and be selected.
5. Move the volume slider and let go. The sound should play at the new level.
6. **Add change** → set "From 10 min every 1 min". The timeline should show tighter ticks after 10:00.
7. Close the window with the X. The tray icon should stay. Right-click it and check the status line and **Test sound**.

## 2. Memory check (2 min)

1. With the settings window closed, open **Task Manager → Details** and sort by name.
2. Find `minimap-nudge.exe` and note its **Memory (active private working set)**.
3. Check whether any `msedgewebview2.exe` processes are left. They belong to the settings window and should disappear a few seconds after you close it.
4. Open settings from the tray and compare. The WebView2 processes come back while the window is open, and that's expected.

## 3. A real game

1. Leave the app in the tray and play a normal Summoner's Rift game, or a Practice Tool session (it uses the same API).
2. Once you're loaded in, the tray icon should turn teal, and the tooltip should read "In game 0:45 · next ping 3:00".
3. You should hear a ping at 3:00, 6:00 and 9:00, then every minute from 10:00 if you added that change.
4. After the game, right-click the tray icon → **Open log folder** → `minimap-nudge.log`. It should show:

   ```
   game detected: CLASSIC at 0:12
   ping at 3:00 (due 3:00)
   ping at 6:00 (due 6:00)
   ...
   game ended
   ```

5. Optionally, play an ARAM with "Only on Summoner's Rift" ticked. The log should show the game detected but no pings.

## If something's off

| What you see | Likely cause and fix |
|---|---|
| The settings window is blank, or the exe won't start | WebView2 is missing (rare on Windows 10/11). Use the installer, which fetches it. |
| No pings, and the log shows no "game detected" line | Something is blocking `127.0.0.1:2999`. Try Practice Tool, and check that antivirus isn't blocking local HTTPS. |
| "audio: no output device" in the log | Windows has no default playback device set. |
| Something else | Send me the log file. |
