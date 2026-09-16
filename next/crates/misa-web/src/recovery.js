// Expired local views may still have browser-owned drafts. Recovery never sends.
(function (global) {
  function start(doc, browser) {
    var target = doc.getElementById("recovery");
    var instance = doc.body.dataset.expiredView;
    if (!target || !instance) return;
    target.replaceChildren();
    function note(text) { var p = doc.createElement("p"); p.textContent = text; target.appendChild(p); }
    var storage, key, state;
    try {
      storage = browser.localStorage;
      key = storage.getItem("misa.view." + instance);
      // Older clients saved the instance in their memory key but had no index.
      if (!key) {
        for (var i = 0; i < storage.length; i++) {
          var candidate = storage.key(i);
          if (candidate && candidate.startsWith("misa.preferences.") && candidate.endsWith(":" + instance)) {
            key = candidate.slice("misa.preferences.".length); break;
          }
        }
      }
      if (key) state = JSON.parse(storage.getItem("misa.preferences." + key));
    } catch (_) { note("Browser draft storage is unavailable. No command has been sent."); return; }
    if (!state || typeof state !== "object") { note("No draft was retained for this view in this browser."); return; }
    function draft(title, text) {
      var label = doc.createElement("label"); label.textContent = title;
      var field = doc.createElement("textarea"); field.readOnly = true; field.value = text;
      label.appendChild(field); target.appendChild(label);
      var select = doc.createElement("button"); select.textContent = "Select text";
      select.addEventListener("click", function () { field.focus(); field.select(); }); target.appendChild(select);
    }
    var count = 0;
    if (typeof state.draft === "string" && state.draft) { draft("Unsent draft", state.draft); count++; }
    if (Array.isArray(state.recoveries)) state.recoveries.slice(0, 16).forEach(function (entry) {
      if (!entry || typeof entry.text !== "string" || !entry.text) return;
      draft(typeof entry.reason === "string" ? entry.reason : "Unconfirmed submission", entry.text); count++;
    });
    note(count ? "Nothing has been resent. Check the current session before copying an unconfirmed submission into a new view." : "No unsent draft or unconfirmed submission was retained for this view.");
  }
  global.MisaRecovery = { start: start };
  if (global.document) start(global.document, global);
})(globalThis);
