// Uranium's half of its tabs in derisk's command palette: every window's
// tabs go to uranium-tabs (src/uranium-tabs), which Chromium starts as this
// extension's native messaging host and which registers them with derisk
// as the window's menus. A tab picked in the palette or the top bar comes
// back as a command.
//
// The "key" in manifest.json fixes this extension's id, which the host's
// manifest names as the one extension allowed to start it. Private windows
// are left out: the extension does not run in them unless the user allows
// it, so their tabs never reach the palette.

const HOST = "org.losos.uranium_tabs";

let port = null;
let timer = null;

// The port is opened again after any event that finds it closed, which
// also wakes this service worker; an open port keeps it running. Outside
// derisk, or with the host missing, the port closes at once and nothing
// else happens.
function connect() {
  if (port) return;
  port = chrome.runtime.connectNative(HOST);
  port.onMessage.addListener(perform);
  port.onDisconnect.addListener(() => {
    port = null;
  });
}

// Tab events come in bursts (closing a window removes every tab in it), so
// the snapshot goes out once they settle.
function changed() {
  connect();
  clearTimeout(timer);
  timer = setTimeout(send, 100);
}

async function send() {
  if (!port) return;
  const windows = await chrome.windows.getAll({ populate: true, windowTypes: ["normal"] });
  port.postMessage({
    windows: windows.map((w) => ({
      id: w.id,
      tabs: w.tabs.map((t) => ({
        id: t.id,
        title: t.title || t.url || "New Tab",
        active: t.active,
      })),
    })),
  });
}

async function perform(message) {
  switch (message.command) {
    case "activate": {
      const tab = await chrome.tabs.update(message.tab, { active: true });
      await chrome.windows.update(tab.windowId, { focused: true });
      break;
    }
    case "close":
      await chrome.tabs.remove(message.tab);
      break;
    case "new":
      await chrome.tabs.create({ windowId: message.window });
      break;
  }
}

chrome.runtime.onStartup.addListener(changed);
chrome.runtime.onInstalled.addListener(changed);
chrome.windows.onCreated.addListener(changed);
chrome.windows.onRemoved.addListener(changed);
for (const event of [
  chrome.tabs.onCreated,
  chrome.tabs.onRemoved,
  chrome.tabs.onActivated,
  chrome.tabs.onMoved,
  chrome.tabs.onAttached,
  chrome.tabs.onDetached,
  chrome.tabs.onReplaced,
]) {
  event.addListener(changed);
}
chrome.tabs.onUpdated.addListener((_id, change) => {
  if ("title" in change || "url" in change) changed();
});
connect();
changed();
