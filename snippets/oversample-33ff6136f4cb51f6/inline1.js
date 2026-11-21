export function language_name(code) {
    try { return new Intl.DisplayNames(['en'], { type: 'language' }).of(code) || code; }
    catch (e) { return code; }
}