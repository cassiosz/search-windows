// What the service worker sees, kept for the popup to read back.
const has = name => typeof chrome[name] === 'object' && chrome[name] !== null;
const seen = {
  ran: true,
  apis: Object.fromEntries(['tabs', 'windows', 'storage', 'alarms', 'scripting', 'declarativeNetRequest', 'contextMenus', 'action', 'webRequest', 'notifications', 'commands', 'sidePanel'].map(n => [n, has(n)])),
};
chrome.storage.local.set({ sw: seen });
chrome.runtime.onMessage.addListener((message, sender, reply) => {
  reply({ pong: message, tab: sender.tab ? { id: sender.tab.id, url: sender.tab.url } : null });
});
