// Supply local choices before the host opens any content observation.
(function (global) {
  function start(doc, browser) {
    doc.addEventListener("submit", function (event) {
      var form = event.target;
      var action = form.getAttribute("action");
      if (action !== "/choose" && action !== "/lifecycle") return;
      if ((form.getAttribute("method") || "get").toLowerCase() !== "post") return;
      var daemon = form.elements.namedItem("daemon");
      var session = form.elements.namedItem("session") || form.elements.namedItem("field.id");
      if (!daemon || !session) return;
      var value = "{}";
      try { value = browser.localStorage.getItem("misa.presentations." + daemon.value + ":" + session.value) || "{}"; } catch (_) {}
      var field = form.elements.namedItem("preferences");
      if (!field) { field = doc.createElement("input"); field.type = "hidden"; field.name = "preferences"; form.appendChild(field); }
      field.value = value;
    });
  }
  global.MisaOpening = { start: start };
  if (global.document) start(global.document, global);
})(globalThis);
