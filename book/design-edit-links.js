// The design pages are written from docs/ when the book is built, so their "Suggest an edit" link
// has to open the file in docs/, not a book/src/design/ copy that is never committed.
document.querySelectorAll('a[href*="/book/src/design/"]').forEach(function (link) {
  link.href = link.href.replace('/book/src/design/', '/docs/');
});
