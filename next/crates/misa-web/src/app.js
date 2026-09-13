// The whole of the browser frontend's script.
//
// It does one thing: replace the main region when the server says the view
// changed. Everything else a page does here — submitting, disclosing, laying out —
// is HTML, and the form works with this file absent. If this file ever grows a
// render function, the frontend has moved and the architecture has not.
(function () {
  var main = document.getElementById("main");
  if (!main || typeof EventSource === "undefined") return;
  var stream = new EventSource("/events");
  stream.onmessage = function (message) {
    main.innerHTML = message.data;
    // Keep the reader where they were: a stream that jumps to the top on every
    // token is worse than no stream at all.
    if (document.activeElement && document.activeElement.name === "prompt") return;
    window.scrollTo({ top: document.body.scrollHeight });
  };
})();
