// Marks the page, then asks the service worker: both are read by the bench.
document.documentElement.dataset.probe = 'cs';
chrome.runtime.sendMessage('ping').then(
  answer => { document.documentElement.dataset.probeSw = JSON.stringify(answer); },
  error => { document.documentElement.dataset.probeSw = 'error: ' + error.message; },
);
