// Private forms have no persistent draft storage and no composer handlers.
(function (global) {
  function start(doc, EventSource) {
    var form = doc.getElementById("private-request");
    if (!form || !EventSource) return;
    var fields = new FormData(form);
    var query = new URLSearchParams({ id: fields.get("id"), generation: fields.get("generation") });
    var events = new EventSource("./request/events?" + query);
    var invalid = false;
    function invalidate(message) {
      if (invalid) return;
      invalid = true;
      form.querySelectorAll("input:not([type=hidden]), textarea").forEach(function (field) { field.value = ""; });
      form.querySelectorAll("input, textarea, button, select").forEach(function (field) { field.disabled = true; });
      doc.getElementById("request-status").textContent = message;
      events.close();
    }
    events.onmessage = function (event) {
      if (event.data === "current" || event.data === "opening") return;
      invalidate(event.data === "resolved" ? "This request has been resolved. You can return to the session." : "This form is no longer current. Reopen the request before responding.");
    };
    events.onerror = function () { invalidate("Request status is unavailable. Reopen the request before responding."); };
    form.addEventListener("submit", function () { events.close(); });
    return { close: function () { events.close(); } };
  }
  global.MisaRequest = { start: start };
  if (global.document) start(global.document, global.EventSource);
})(typeof window === "undefined" ? globalThis : window);
