/* Page wiring only — all generation logic lives in wizard.js (pure). */
/* Externalised from an inline <script> so the site's strict CSP
   (script-src 'self', no 'unsafe-inline') allows it to run. */
(function () {
  "use strict";
  var W = window.KcpWizard;
  if (!W) return;

  var patternList = document.getElementById("pattern-list");
  var featureList = document.getElementById("feature-list");
  var nameInput = document.getElementById("app-name");
  var output = document.getElementById("output");
  var copyBtn = document.getElementById("copy-btn");
  var downloadBtn = document.getElementById("download-btn");
  var copyNote = document.getElementById("copy-note");

  var state = { pattern: W.PATTERNS[0].id, features: {}, name: "" };

  function currentPattern() {
    for (var i = 0; i < W.PATTERNS.length; i++) {
      if (W.PATTERNS[i].id === state.pattern) return W.PATTERNS[i];
    }
    return W.PATTERNS[0];
  }
  function currentFileName() {
    var pat = currentPattern();
    return W.sanitizeName(state.name, pat.defaultName) + ".portrait";
  }
  function render() {
    var pat = currentPattern();
    var feats = [];
    for (var i = 0; i < pat.features.length; i++) {
      if (state.features[pat.features[i].id]) feats.push(pat.features[i].id);
    }
    output.textContent = W.generate(pat.id, feats, state.name);
    nameInput.placeholder = pat.defaultName;
    downloadBtn.textContent = "Download " + currentFileName();
  }
  function buildPatterns() {
    patternList.textContent = "";
    W.PATTERNS.forEach(function (p) {
      var label = document.createElement("label");
      label.className = "pattern-option" + (p.id === state.pattern ? " selected" : "");
      var input = document.createElement("input");
      input.type = "radio"; input.name = "pattern"; input.value = p.id;
      input.checked = p.id === state.pattern;
      input.addEventListener("change", function () {
        state.pattern = p.id; state.features = {};
        buildPatterns(); buildFeatures(); render();
      });
      var strong = document.createElement("span");
      strong.className = "p-label"; strong.textContent = p.label;
      var desc = document.createElement("span");
      desc.className = "p-desc"; desc.textContent = p.desc;
      label.appendChild(input); label.appendChild(strong); label.appendChild(desc);
      patternList.appendChild(label);
    });
  }
  function buildFeatures() {
    featureList.textContent = "";
    var pat = currentPattern();
    pat.features.forEach(function (f) {
      var label = document.createElement("label");
      label.className = "feature-option";
      var input = document.createElement("input");
      input.type = "checkbox"; input.value = f.id;
      input.checked = !!state.features[f.id];
      input.addEventListener("change", function () {
        state.features[f.id] = input.checked; render();
      });
      var strong = document.createElement("span");
      strong.className = "f-label"; strong.textContent = f.label;
      var inv = document.createElement("code");
      inv.className = "f-inv"; inv.textContent = f.invariant;
      var desc = document.createElement("span");
      desc.className = "f-desc"; desc.textContent = f.desc;
      label.appendChild(input); label.appendChild(strong); label.appendChild(inv); label.appendChild(desc);
      featureList.appendChild(label);
    });
  }
  nameInput.addEventListener("input", function () { state.name = nameInput.value; render(); });
  function flash(msg) {
    copyNote.textContent = msg;
    window.setTimeout(function () { if (copyNote.textContent === msg) copyNote.textContent = ""; }, 2500);
  }
  copyBtn.addEventListener("click", function () {
    var text = output.textContent;
    function fallback() {
      var ta = document.createElement("textarea");
      ta.value = text; ta.setAttribute("readonly", "");
      ta.style.position = "fixed"; ta.style.left = "-999rem";
      document.body.appendChild(ta); ta.select();
      var ok = false;
      try { ok = document.execCommand("copy"); } catch (e) {}
      document.body.removeChild(ta);
      flash(ok ? "Copied." : "Copy failed — select the text manually.");
    }
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(function () { flash("Copied."); }, fallback);
    } else { fallback(); }
  });
  downloadBtn.addEventListener("click", function () {
    var blob = new Blob([output.textContent], { type: "text/plain" });
    var url = URL.createObjectURL(blob);
    var a = document.createElement("a");
    a.href = url; a.download = currentFileName();
    document.body.appendChild(a); a.click(); document.body.removeChild(a);
    window.setTimeout(function () { URL.revokeObjectURL(url); }, 1000);
  });

  buildPatterns(); buildFeatures(); render();
})();
