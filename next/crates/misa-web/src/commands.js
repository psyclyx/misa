// Completion enhances local command fields. It never changes shared selection
// or submits a command; stale replies cannot replace newer candidate choices.
(function (global) {
  function start(doc, browser) {
    if (!browser.fetch || doc.misaCompletions) return;
    doc.misaCompletions = true;
    var pending = new Map();
    doc.addEventListener("input", function (event) {
      var field = event.target;
      if (!field.dataset || !field.dataset.source) return;
      var old = pending.get(field);
      if (old) { clearTimeout(old.timer); old.controller.abort(); pending.delete(field); }
      if (pending.size >= 4) return;
      var controller = new AbortController(), prefix = field.value;
      var entry = { controller: controller, timer: setTimeout(async function () {
        var status = doc.getElementById(field.getAttribute("aria-describedby"));
        try {
          var response = await browser.fetch("./completions?" + new URLSearchParams({ source: field.dataset.source, q: prefix }), { signal: controller.signal });
          var result = await response.json();
          if (!response.ok) throw Error(result.error || "Choices unavailable");
          if (pending.get(field) !== entry || field.value !== prefix || !field.isConnected) return;
          var list = doc.getElementById(field.getAttribute("list"));
          if (!list) return;
          list.replaceChildren();
          result.items.forEach(function (choice) {
            var option = doc.createElement("option"); option.value = choice.value;
            option.textContent = choice.label + (choice.detail ? " · " + choice.detail : ""); list.appendChild(option);
          });
          if (status) status.textContent = result.truncated ? "More matches available; keep typing." : "";
        } catch (error) {
          if (!controller.signal.aborted && pending.get(field) === entry && status) status.textContent = error.message;
        } finally { if (pending.get(field) === entry) pending.delete(field); }
      }, 120) };
      pending.set(field, entry);
    });
  }
  global.MisaCommands = { start: start };
  if (global.document) start(global.document, global);
})(typeof window === "undefined" ? globalThis : window);
