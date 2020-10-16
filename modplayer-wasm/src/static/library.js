var term;

mergeInto(LibraryManager.library, {
    term_writeln: function (str) {
        term.writeln(UTF8ToString(str));
    },
});
