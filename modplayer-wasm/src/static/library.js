let term;

export function set_term(obj) {
    term = obj;
}

export function term_writeln(str) {
    term.writeln(UTF8ToString(str));
}
