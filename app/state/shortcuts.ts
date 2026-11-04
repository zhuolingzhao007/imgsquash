import { keymap } from "nsh";
import { recompress, startCompress } from "./compression";
import { files, pickFiles, pickFolders } from "./queue";
import { settingsOpen } from "./view";

export function bindShortcuts() {
  keymap.bind({
    "mod+o": () => { void pickFiles(); },
    "mod+shift+o": () => { void pickFolders(); },
    "mod+enter": () => {
      if (files().some(file => file.status !== "done")) void startCompress();
      else void recompress();
    },
    "escape": () => { if (settingsOpen()) settingsOpen.set(false); },
  });
}
