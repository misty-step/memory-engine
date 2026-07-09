// Honest response timing for review submissions.
//
// The server renders every review form with a blank responseTimeMs and grades
// a blank value conservatively (it can never rate Easy), so with JavaScript
// off the flow still works and never overstates the learner's speed. This
// script records when the card was presented — script evaluation, right after
// the server-rendered card became visible — and fills in the real
// presentation-to-submit elapsed time at the moment of submission.
(function () {
  "use strict";
  if (!document.querySelector('input[name="responseTimeMs"]')) return;
  var monotonic = window.performance && typeof performance.now === "function";
  var shownAt = monotonic ? performance.now() : Date.now();
  document.addEventListener("submit", function (event) {
    var form = event.target;
    if (!form || !form.querySelector) return;
    var input = form.querySelector('input[name="responseTimeMs"]');
    if (!input) return;
    var elapsed = (monotonic ? performance.now() : Date.now()) - shownAt;
    input.value = String(Math.max(1, Math.round(elapsed)));
  });
})();

// Progressive enhancement for the generation activity log.
//
// The server renders the authoritative list of jobs on every full page load,
// so with JavaScript off (or if this script fails) the page is still correct
// and a normal navigation refreshes it. This script only *enhances* that list:
// it opens an SSE stream and patches a single <li> in place as each job's
// status changes — no framework, no full-list rebuild, no lost scroll.
(function () {
  "use strict";
  if (!("EventSource" in window)) return;

  var list = document.getElementById("me-jobs");

  // The human meta line, kept in sync with `job_meta` in render.rs.
  function metaFor(job) {
    switch (job.status) {
      case "queued":
        return "Queued…";
      case "running":
        return "Generating cards…";
      case "succeeded":
        var n = job.cardCount || 0;
        return n + " " + (n === 1 ? "card" : "cards") + " · scheduled for review";
      case "failed":
        return job.error || "Generation failed — try again.";
      default:
        return "";
    }
  }

  // Build a minimal row when a job arrives that isn't on the page yet (e.g. a
  // capture made in another tab). It carries status + meta only — the retry
  // control needs a server-issued CSRF token, so a job that fails here gets its
  // Retry button on the next full page load (the list is server-authoritative).
  function createRow(job) {
    var li = document.createElement("li");
    li.className = "me-job";
    li.dataset.jobId = job.id;
    li.innerHTML =
      '<span class="me-job-glyphs" aria-hidden="true">' +
      '<span class="g-queued"></span><span class="g-running"><span class="me-spinner"></span></span>' +
      '<span class="g-succeeded"></span><span class="g-failed"></span></span>' +
      '<div class="me-job-body"><p class="me-job-title"></p><p class="me-job-meta"></p></div>';
    li.querySelector(".me-job-title").textContent = job.title || "New material";
    return li;
  }

  function apply(job) {
    if (!list) return;
    var li = list.querySelector('li[data-job-id="' + cssEscape(job.id) + '"]');
    if (!li) {
      li = createRow(job);
      list.insertBefore(li, list.firstChild);
    }
    li.dataset.status = job.status;
    var meta = li.querySelector(".me-job-meta");
    if (meta) meta.textContent = metaFor(job);
  }

  function cssEscape(value) {
    return window.CSS && CSS.escape ? CSS.escape(value) : String(value).replace(/"/g, '\\"');
  }

  var source = new EventSource("/app/jobs/events");
  // EventSource reconnects automatically; no manual retry needed.
  source.addEventListener("job", function (event) {
    try {
      apply(JSON.parse(event.data));
    } catch (err) {
      /* ignore a malformed frame; the next event or reload corrects it */
    }
  });
})();
