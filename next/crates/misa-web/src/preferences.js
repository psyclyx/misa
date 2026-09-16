// One atomic storage key per choice: independent tabs never replace a profile.
(function (global) {
  var prefix = "misa.presentation.choice.";
  function read(storage, context) {
    var choices = Object.create(null);
    try {
      var legacy = JSON.parse(storage.getItem("misa.presentations." + context));
      if (legacy && typeof legacy === "object") Object.entries(legacy).slice(0, 128).forEach(function (entry) {
        if (typeof entry[1] === "string") choices[entry[0]] = entry[1];
      });
      for (var index = 0; index < storage.length; index++) {
        var key = storage.key(index);
        if (!key || !key.startsWith(prefix)) continue;
        var identity;
        try { identity = JSON.parse(key.slice(prefix.length)); } catch (_) { continue; }
        if (!Array.isArray(identity) || identity.length !== 2 || identity[0] !== context || typeof identity[1] !== "string") continue;
        if (!Object.prototype.hasOwnProperty.call(choices, identity[1]) && Object.keys(choices).length >= 128) continue;
        var choice = storage.getItem(key);
        if (typeof choice === "string") choices[identity[1]] = choice;
      }
    } catch (_) {}
    return choices;
  }
  function write(storage, context, id, choice) {
    storage.setItem(prefix + JSON.stringify([context, id]), choice);
  }
  global.MisaPreferences = { read: read, write: write };
})(globalThis);
