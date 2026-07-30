/* Freally MIDI Master — documentation search.
   © 2026 Mike Weaver — All Rights Reserved.

   No dependencies and no network: the index is a script tag, the match is a
   scored substring pass over fifteen entries. Anything cleverer would be a
   build step for a page that fits on one screen. */

(function () {
  'use strict';

  var box = document.getElementById('q');
  var list = document.getElementById('results');
  if (!box || !list || !window.SEARCH_INDEX) return;

  /** Rank an entry against the query's terms. 0 means "not a match". */
  function score(entry, terms) {
    var title = entry.title.toLowerCase();
    var body = entry.body.toLowerCase();
    var total = 0;

    for (var i = 0; i < terms.length; i += 1) {
      var term = terms[i];
      // Every term has to appear somewhere, so a two-word query narrows
      // rather than widens — the opposite is what makes small search boxes
      // feel broken.
      if (title.indexOf(term) === -1 && body.indexOf(term) === -1) return 0;
      if (title.indexOf(term) === 0) total += 8;
      else if (title.indexOf(term) !== -1) total += 5;
      else total += 1;
    }

    return total;
  }

  function render(matches, query) {
    list.innerHTML = '';

    if (!query) {
      list.hidden = true;
      return;
    }

    if (matches.length === 0) {
      var empty = document.createElement('li');
      empty.className = 'search-empty';
      empty.textContent = 'Nothing matches “' + query + '”.';
      list.appendChild(empty);
      list.hidden = false;
      return;
    }

    matches.forEach(function (entry) {
      var li = document.createElement('li');
      var a = document.createElement('a');
      a.href = '#' + entry.id;
      a.textContent = entry.title;
      li.appendChild(a);
      list.appendChild(li);
    });

    list.hidden = false;
  }

  function run() {
    var query = box.value.trim();
    var terms = query.toLowerCase().split(/\s+/).filter(Boolean);

    if (terms.length === 0) {
      render([], '');
      return;
    }

    var matches = window.SEARCH_INDEX.map(function (entry) {
      return { entry: entry, rank: score(entry, terms) };
    })
      .filter(function (row) {
        return row.rank > 0;
      })
      .sort(function (a, b) {
        return b.rank - a.rank;
      })
      .map(function (row) {
        return row.entry;
      });

    render(matches, query);
  }

  box.addEventListener('input', run);

  // Escape clears, because a search box that traps you is worse than none.
  box.addEventListener('keydown', function (event) {
    if (event.key === 'Escape') {
      box.value = '';
      run();
    }
  });
})();
