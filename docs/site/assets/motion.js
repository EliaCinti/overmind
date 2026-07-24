/* ==========================================================================
   Overmind — motion.js
   Calm, self-contained motion for the static site. No libraries, no CDNs.
   Everything degrades gracefully: if JS never runs, nothing is hidden; if the
   visitor prefers reduced motion, we bail out and leave instant final states.
   ========================================================================== */
(function () {
  'use strict';

  var root = document.documentElement;
  var reduce = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  // Signal to CSS that JS is live. Reveal hidden-states are gated on `.motion`,
  // so content is only ever hidden when we can guarantee we'll reveal it.
  if (reduce) {
    root.classList.add('motion-reduced');
    return; // honor the user — no reveals, no tilt, no scroll-spy.
  }
  root.classList.add('motion');

  /* ---- 1. Scroll-reveal ---------------------------------------------------
     Curated selectors get a gentle fade/slide-up as they enter the viewport.
     Children of a grid are staggered so a row breathes in, not all at once. */
  var revealSelectors = [
    'section > h2', 'section > .lede', 'section > .eyebrow',
    '.card', '.feature', '.step', '.teaser', '.link-card',
    '.wadachi-band', '.callout', 'figure.framed', '.quickstart pre'
  ];
  var targets = [];
  document.querySelectorAll(revealSelectors.join(',')).forEach(function (el) {
    // Skip anything already inside a revealed ancestor to avoid double motion.
    if (el.closest('[data-reveal-group]')) return;
    el.setAttribute('data-reveal', '');
    targets.push(el);
  });

  // Stagger siblings that share a grid/flex parent.
  document.querySelectorAll('.grid-3, .grid-2, .features, .links-grid, .steps').forEach(function (group) {
    var kids = group.querySelectorAll(':scope > [data-reveal]');
    kids.forEach(function (kid, i) {
      kid.style.setProperty('--reveal-delay', (i * 70) + 'ms');
    });
  });

  if ('IntersectionObserver' in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) {
          entry.target.classList.add('is-visible');
          io.unobserve(entry.target);
        }
      });
    }, { root: null, rootMargin: '0px 0px -8% 0px', threshold: 0.12 });
    targets.forEach(function (el) { io.observe(el); });
  } else {
    // No observer? Show everything immediately.
    targets.forEach(function (el) { el.classList.add('is-visible'); });
  }

  /* ---- 2. Pointer tilt ----------------------------------------------------
     A light 3D tilt + glare that follows the cursor. Fine pointers only, so
     touch devices are untouched. Kept subtle: max ~5deg. */
  var finePointer = window.matchMedia && window.matchMedia('(hover: hover) and (pointer: fine)').matches;
  if (finePointer) {
    var tiltEls = document.querySelectorAll('.card, .feature, .link-card, figure.framed');
    var MAX = 5; // degrees
    tiltEls.forEach(function (el) {
      el.classList.add('tilt');
      var raf = null;
      function onMove(e) {
        if (raf) return;
        raf = requestAnimationFrame(function () {
          raf = null;
          var r = el.getBoundingClientRect();
          var px = (e.clientX - r.left) / r.width;   // 0..1
          var py = (e.clientY - r.top) / r.height;    // 0..1
          var rx = (0.5 - py) * (MAX * 2);
          var ry = (px - 0.5) * (MAX * 2);
          el.style.setProperty('--rx', rx.toFixed(2) + 'deg');
          el.style.setProperty('--ry', ry.toFixed(2) + 'deg');
          el.style.setProperty('--gx', (px * 100).toFixed(1) + '%');
          el.style.setProperty('--gy', (py * 100).toFixed(1) + '%');
        });
      }
      function reset() {
        el.style.setProperty('--rx', '0deg');
        el.style.setProperty('--ry', '0deg');
      }
      el.addEventListener('pointermove', onMove);
      el.addEventListener('pointerleave', reset);
    });
  }

  /* ---- 3. Scroll-spy for in-page anchors ----------------------------------
     Underline the nav link whose section is in view (home page has #pitch,
     #wadachi, #quickstart, #links). Cross-page links are left alone. */
  var anchorLinks = Array.prototype.filter.call(
    document.querySelectorAll('nav.site .links a[href^="#"]'),
    function (a) { return a.getAttribute('href').length > 1; }
  );
  if (anchorLinks.length && 'IntersectionObserver' in window) {
    var byId = {};
    anchorLinks.forEach(function (a) {
      var id = a.getAttribute('href').slice(1);
      var sec = document.getElementById(id);
      if (sec) byId[id] = a;
    });
    var spy = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (!entry.isIntersecting) return;
        var link = byId[entry.target.id];
        if (!link) return;
        anchorLinks.forEach(function (a) { a.classList.remove('spy'); });
        link.classList.add('spy');
      });
    }, { rootMargin: '-45% 0px -50% 0px', threshold: 0 });
    Object.keys(byId).forEach(function (id) { spy.observe(document.getElementById(id)); });
  }
})();
