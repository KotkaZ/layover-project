// Added to the development build only, which is published from main at /dev/: these pages can
// describe what no released Layover does yet, so each one says so and links to the release's.
(function () {
  var main = document.querySelector('main');
  if (!main) {
    return;
  }
  var release = location.href.split('/dev/')[0] + '/';
  var note = document.createElement('div');
  note.setAttribute('role', 'note');
  note.style.cssText =
    'border-left: 4px solid #e6a23c; padding: 0.6em 1em; margin: 0 0 1.5em; ' +
    'background: rgba(230, 162, 60, 0.12);';
  note.innerHTML =
    'These pages describe <strong>main</strong>, which may hold changes no released Layover has ' +
    'yet. <a href="' + release + '">The pages for the latest release</a>.';
  main.insertBefore(note, main.firstChild);
})();
