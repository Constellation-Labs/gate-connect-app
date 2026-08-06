# Post-install: point Gate Connect at staging

This guide is for pointing Gate Connect at the **staging** gateway without
using the command line. 


**Do this first:** Launch Gate Connect once and sign in. This creates the
settings file and stores your API key securely in the system keychain. Then
**quit the app** before editing - it reads the file at startup.

Your API key is *not* in this file, so these edits never touch it.

### macOS

1. Open **Finder**.
2. In the menu bar choose **Go → Go to Folder…** (or press
   <kbd>Shift</kbd>+<kbd>Cmd</kbd>+<kbd>G</kbd>).
3. Paste this and press Return:
   ```
   ~/Library/Application Support/Gate Connect
   ```
4. Right-click **account.json** → **Open With → TextEdit**.
5. Change the address inside the quotes to the staging URL so the file reads:
   ```
   {
     "gateway_base_url": "https://gateway-staging.constellationgate.ai"
   }
   ```
6. Save (<kbd>Cmd</kbd>+<kbd>S</kbd>) and close TextEdit.
7. Relaunch Gate Connect.

### Windows

1. Open **File Explorer**.
2. Click the address bar, paste this, and press Enter:
   ```
   %LOCALAPPDATA%\Gate Connect
   ```
3. Right-click **account.json** → **Open with → Notepad**.
4. Change the address inside the quotes to the staging URL so the file reads:
   ```
   {
     "gateway_base_url": "https://gateway-staging.constellationgate.ai"
   }
   ```
5. Save (<kbd>Ctrl</kbd>+<kbd>S</kbd>) and close Notepad.
6. Relaunch Gate Connect.

### Linux

1. Open your file manager (Files / Nautilus, Dolphin, etc.).
2. Go to this folder (paste into the location bar - in Files press
   <kbd>Ctrl</kbd>+<kbd>L</kbd> first):
   ```
   ~/.local/share/Gate Connect
   ```
3. Open **account.json** in a plain-text editor (Text Editor / gedit / Kate).
4. Change the address inside the quotes to the staging URL so the file reads:
   ```
   {
     "gateway_base_url": "https://gateway-staging.constellationgate.ai"
   }
   ```
5. Save and relaunch Gate Connect.

---

### Tips

- **Keep the quotes and the curly braces.** Only swap the web address between
  the quotation marks.
- On macOS, if TextEdit shows a styled (rich text) document, choose
  **Format → Make Plain Text** before saving so the file stays plain text.
- To confirm it worked: open the menu-bar popover, open **Settings** (the gear),
  and check the workspace address now points at
  `gateway-staging.constellationgate.ai`.
