//! Xeno-canto group names typed in a species search, such as "bat" in
//! "gould bat", in English, Latin and other common languages.

/// Words and phrases for each XC group, folded with [`fold`]. Word forms that
/// are a genus or part of many species names in another group (Italian
/// "pipistrello", which is also the pipistrelles' name) are left out.
#[rustfmt::skip]
const GROUP_WORDS: &[(&str, &[&str])] = &[
    (
        "bats",
        &[
            "bat", "bats", "chiroptera", "chiropteran", "chiropterans",
            // de, fr, es, it, nl, pt
            "fledermaus", "fledermause", "chauve-souris", "chauves-souris", "murcielago",
            "murcielagos", "chirottero", "chirotteri", "vleermuis", "vleermuizen", "morcego",
            "morcegos",
            // sv, da, no, fi, pl, cs, hu, ro
            "fladdermus", "fladdermoss", "flagermus", "flagermusene", "flaggermus", "lepakko",
            "lepakot", "nietoperz", "nietoperze", "netopyr", "netopyri", "denever", "liliac",
            "lilieci",
            // ru, uk, ja, zh, ko
            "летучая мышь", "летучие мыши", "рукокрылые", "кажан", "кажани", "コウモリ",
            "こうもり", "蝙蝠", "박쥐",
        ],
    ),
    (
        "birds",
        &[
            "bird", "birds", "aves",
            "vogel", "vogels", "oiseau", "oiseaux", "pajaro", "pajaros", "uccello", "uccelli",
            "passaro", "passaros",
            "fagel", "faglar", "fugl", "fugle", "fugler", "lintu", "linnut", "ptak", "ptaki",
            "ptaci", "madar", "madarak", "pasare", "pasari",
            "птица", "птицы", "птах", "птахи", "鳥", "鸟", "とり", "새",
        ],
    ),
    (
        "frogs",
        &[
            "frog", "frogs", "toad", "toads", "anura", "anuran", "anurans", "amphibian",
            "amphibians",
            "frosch", "frosche", "krote", "kroten", "grenouille", "grenouilles", "crapaud",
            "crapauds", "rana", "ranas", "sapo", "sapos", "rospo", "rospi", "kikker", "kikkers",
            "pad", "padden",
            "groda", "grodor", "padda", "paddor", "fro", "froer", "frosk", "sammakko",
            "sammakot", "zaba", "zaby", "zabka", "beka", "broasca", "broaste",
            "лягушка", "лягушки", "жаба", "жабы", "カエル", "かえる", "蛙", "青蛙", "개구리",
        ],
    ),
    (
        "grasshoppers",
        &[
            "grasshopper", "grasshoppers", "cricket", "crickets", "bush-cricket",
            "bush-crickets", "bush cricket", "bush crickets", "bushcricket", "bushcrickets",
            "katydid", "katydids", "locust", "locusts", "orthoptera", "orthopteran",
            "orthopterans",
            "heuschrecke", "heuschrecken", "grille", "grillen", "sauterelle", "sauterelles",
            "criquet", "criquets", "grillon", "grillons", "saltamontes", "grillo", "grillos",
            "grilli", "cavalletta", "cavallette", "sprinkhaan", "sprinkhanen", "krekel",
            "krekels", "gafanhoto", "gafanhotos", "grilo", "grilos",
            "grashoppa", "grashoppor", "syrsa", "syrsor", "graeshoppe", "graeshopper",
            "gresshoppe", "gresshopper", "heinasirkka", "heinasirkat", "sirkka", "sirkat",
            "konik polny", "koniki polne", "pasikonik", "pasikoniki", "swierszcz",
            "swierszcze", "kobylka", "kobylky", "cvrcek", "cvrcci", "saska", "tucsok",
            "lacusta", "greier", "greiere",
            "кузнечик", "кузнечики", "сверчок", "сверчки", "саранча", "прямокрылые", "バッタ",
            "キリギリス", "コオロギ", "蝗虫", "蚱蜢", "蟋蟀", "螽斯", "메뚜기", "귀뚜라미",
        ],
    ),
    (
        "land mammals",
        &[
            "mammal", "mammals", "land mammal", "land mammals", "mammalia",
            "saugetier", "saugetiere", "mammifere", "mammiferes", "mamifero", "mamiferos",
            "mammifero", "mammiferi", "zoogdier", "zoogdieren",
            "daggdjur", "pattedyr", "nisakas", "nisakkaat", "ssak", "ssaki", "savec", "savci",
            "emlos", "emlosok", "mamifer", "mamifere",
            "млекопитающее", "млекопитающие", "ссавець", "ссавці", "哺乳類", "哺乳类",
            "哺乳動物", "哺乳动物", "포유류",
        ],
    ),
];

/// Lower case, with accents removed from Latin letters ("Fledermäuse" ->
/// "fledermause"), so typed words compare with [`GROUP_WORDS`].
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => out.push('a'),
            'ç' | 'ć' | 'č' => out.push('c'),
            'ď' => out.push('d'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' => out.push('i'),
            'ł' => out.push('l'),
            'ñ' | 'ń' | 'ň' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => out.push('o'),
            'ř' => out.push('r'),
            'ś' | 'š' | 'ș' | 'ş' => out.push('s'),
            'ť' | 'ț' | 'ţ' => out.push('t'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'ź' | 'ż' | 'ž' => out.push('z'),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            'ß' => out.push_str("ss"),
            _ => out.push(c),
        }
    }
    out
}

/// The XC group that `phrase` names ("Bats", "Fledermaus", "コウモリ" ->
/// "bats"), or `None`.
pub fn group_named(phrase: &str) -> Option<&'static str> {
    let key = fold(phrase.trim());
    GROUP_WORDS
        .iter()
        .find(|(_, words)| words.contains(&key.as_str()))
        .map(|(group, _)| *group)
}

/// A group named in a species search, and the rest of the search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupInQuery {
    pub group: &'static str,
    /// The search without the group's word, such as "gould" for "gould bat".
    pub rest: String,
}

/// Finds a group named in a species search: "gould bat" gives "bats" and
/// "gould". `None` when no group is named, when the search is only the
/// group's name ("bats" is searched as a name), or when it names two groups
/// ("cricket frog").
pub fn split_group(query: &str) -> Option<GroupInQuery> {
    let words: Vec<&str> = query.split_whitespace().collect();
    // Commas and full stops typed after a word are not part of it.
    let bare = |w: &str| w.trim_end_matches([',', '.', ';']).to_string();
    let mut found: Option<(&'static str, usize, usize)> = None;
    let mut i = 0;
    while i < words.len() {
        // Two-word names first ("land mammals", "летучая мышь").
        let pair = (i + 1 < words.len())
            .then(|| group_named(&format!("{} {}", words[i], bare(words[i + 1]))))
            .flatten();
        let (group, len) = match pair {
            Some(g) => (Some(g), 2),
            None => (group_named(&bare(words[i])), 1),
        };
        if let Some(g) = group {
            match found {
                Some((other, _, _)) if other != g => return None,
                Some(_) => {}
                None => found = Some((g, i, len)),
            }
        }
        i += len;
    }
    let (group, start, len) = found?;
    let rest: Vec<&str> = words
        .iter()
        .enumerate()
        .filter(|(i, _)| *i < start || *i >= start + len)
        .map(|(_, w)| *w)
        .collect();
    if rest.is_empty() {
        return None;
    }
    Some(GroupInQuery {
        group,
        rest: rest.join(" "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(q: &str) -> Option<(&'static str, String)> {
        split_group(q).map(|g| (g.group, g.rest))
    }

    #[test]
    fn group_words_are_folded() {
        for (_, words) in GROUP_WORDS {
            for w in *words {
                assert_eq!(fold(w), *w, "{w} is not folded");
            }
        }
    }

    #[test]
    fn group_words_name_one_group() {
        for (g, words) in GROUP_WORDS {
            for w in *words {
                assert_eq!(group_named(w), Some(*g), "{w}");
            }
        }
    }

    #[test]
    fn names_in_other_languages() {
        assert_eq!(group_named("Bats"), Some("bats"));
        assert_eq!(group_named("Fledermäuse"), Some("bats"));
        assert_eq!(group_named("murciélago"), Some("bats"));
        assert_eq!(group_named("Vögel"), Some("birds"));
        assert_eq!(group_named("Säugetiere"), Some("land mammals"));
        assert_eq!(group_named("コウモリ"), Some("bats"));
        assert_eq!(group_named("gould"), None);
    }

    #[test]
    fn splits_group_word_from_name() {
        assert_eq!(split("gould bat"), Some(("bats", "gould".into())));
        assert_eq!(split("Bats gould"), Some(("bats", "gould".into())));
        assert_eq!(split("fruit bat,"), Some(("bats", "fruit".into())));
        assert_eq!(
            split("Gould land mammals"),
            Some(("land mammals", "Gould".into()))
        );
        assert_eq!(
            split("летучая мышь ночница"),
            Some(("bats", "ночница".into()))
        );
        assert_eq!(split("gould bat bats"), Some(("bats", "gould bats".into())));
    }

    #[test]
    fn keeps_search_without_one_group() {
        assert_eq!(split("gould"), None);
        assert_eq!(split("bats"), None);
        assert_eq!(split("land mammals"), None);
        // Two groups named: searched as a name.
        assert_eq!(split("cricket frog"), None);
        // Part of a word.
        assert_eq!(split("frogmouth gould"), None);
    }
}
