export function language_autonym(code) {
    try { return new Intl.DisplayNames([code], { type: 'language' }).of(code) || code; }
    catch (e) { return code; }
}