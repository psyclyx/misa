// Overview publications replace only summaries, never local form drafts.
(function (global) {
  function start(doc, Source) {
    var region = doc.getElementById("overview");
    if (!region || !Source) return;
    var status = doc.getElementById("overview-status");
    var source = new Source("/daemons/events");
    source.onmessage = function (event) {
      region.innerHTML = event.data;
      status.textContent = "";
    };
    source.onerror = function () {
      status.textContent = "Overview connection lost; displayed values may be stale.";
      region.querySelectorAll("button").forEach(function (button) { button.disabled = true; });
    };
    return source;
  }
  global.MisaOverview = { start: start };
  if (global.document) start(global.document, global.EventSource);
})(globalThis);
