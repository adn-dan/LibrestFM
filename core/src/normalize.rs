//! Clés de rapprochement : deux écritures d'un même artiste ou titre doivent
//! donner la même clé (casse, accents, ponctuation, « feat. », « Remastered 2011 »),
//! sans fusionner des œuvres différentes (une version live reste distincte).

use regex::Regex;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

/// Version des règles de rapprochement ci-dessous : à augmenter à chaque changement qui
/// modifie des clés. À l'ouverture, une base dont les clés datent d'une autre version les
/// recalcule (Library::rekey). 2 : « Edit » seul n'est plus la même œuvre. 3 : titres
/// recomposés (NFC) avant les mentions de version.
pub const RULES_VERSION: i64 = 3;

/// Mots qui, dans une parenthèse ou après un tiret, désignent la même œuvre.
const SAME_WORK: &str = r"remaster(ed)?|remasteris[ée]+s?|remasterizad[oa]|rimasterizzat[oa]|mono|stereo|radio edit|\d{4} edit|single version|album version|explicit|clean|bonus track|deluxe( edition)?|\d{4} mix|original mix|official (music |lyric )?(video|audio)|clip officiel|audio officiel|lyrics?( video)?|visuali[sz]er|https?://\S+|www\.\S+";

static FEAT_BRACKET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s*[\(\[]\s*(feat\.?|ft\.?|featuring|with)\s[^\)\]]*[\)\]]").unwrap());
static FEAT_BARE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s+(feat\.?|ft\.?|featuring)\s.*$").unwrap());
static SAME_BRACKET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)\s*[\(\[][^\)\]]*\b({SAME_WORK})\b[^\)\]]*[\)\]]")).unwrap()
});
static SAME_DASH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)\s+-\s+[^-]*\b({SAME_WORK})\b.*$")).unwrap());

/// Minuscules, sans accents, sans ponctuation ni espaces. « & » vaut « and ».
pub fn fold(s: &str) -> String {
    // Entités HTML laissées par certains exports (« &amp; ») : décodées d'abord.
    let s = unescape_html(s);
    // Accents retirés seulement sur les lettres latines : sur du japonais, retirer
    // le dakuten changerait le mot (ド → ト). Texte recomposé (NFC) d'abord : une forme
    // décomposée (ト + dakuten, hangul en jamos) rejoint sa forme composée.
    let mut stripped = String::with_capacity(s.len());
    for c in s.nfc() {
        let decomposed: String = c.to_string().nfkd().collect();
        if decomposed.chars().next().is_some_and(|b| b.is_ascii()) {
            stripped.extend(decomposed.chars().filter(|m| !unicode_normalization::char::is_combining_mark(*m)));
        } else {
            // Lettres latines sans décomposition : écrites aussi « oe », « ss »…
            match c {
                'œ' | 'Œ' => stripped.push_str("oe"),
                'æ' | 'Æ' => stripped.push_str("ae"),
                'ß' => stripped.push_str("ss"),
                'ø' | 'Ø' => stripped.push('o'),
                'ł' | 'Ł' => stripped.push('l'),
                'đ' | 'Đ' => stripped.push('d'),
                _ => stripped.extend(c.to_string().nfkc()),
            }
        }
    }
    let lowered = stripped.to_lowercase().replace('&', "and");
    let key: String = lowered.chars().filter(|c| c.is_alphanumeric()).collect();
    if key.is_empty() {
        // Titre fait uniquement de ponctuation (« ... », « ? ») : on garde le texte brut.
        s.trim().to_lowercase()
    } else {
        key
    }
}

pub fn artist_key(name: &str) -> String {
    let name: String = name.nfc().collect();
    fold(&FEAT_BARE.replace(&name, ""))
}

pub fn title_key(title: &str) -> String {
    // Recomposé d'abord : « Remasterisé » décomposé (é = e + accent, comme le renvoie
    // Libre.fm) doit être reconnu par les mentions de version ci-dessous.
    let title: String = title.nfc().collect();
    let t = FEAT_BRACKET.replace_all(&title, "");
    // Mention retirée sauf si elle désigne aussi une autre version (« Remix Edit »).
    let same = |c: &regex::Captures| if LIVE.is_match(&c[0]) { c[0].to_owned() } else { String::new() };
    let t = SAME_BRACKET.replace_all(&t, same);
    let t = SAME_DASH.replace(&t, same);
    let t = FEAT_BARE.replace(&t, "");
    fold(&t)
}

pub fn album_key(title: &str) -> String {
    title_key(title)
}

/// Nettoie une valeur brute : entités HTML décodées (l'API Libre.fm renvoie
/// « Simon &amp; Garfunkel »), espaces superflus, chaîne vide → None.
pub fn clean(s: &str) -> Option<String> {
    let s = unescape_html(s);
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() { None } else { Some(t) }
}

static ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&(amp|lt|gt|quot|apos|#\d{1,7}|#[xX][0-9a-fA-F]{1,6});").unwrap());

/// Décode les entités HTML courantes (`&amp;`, `&#39;`, `&#x27;`…) ; inchangé s'il n'y en a pas.
pub fn unescape_html(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    ENTITY
        .replace_all(s, |c: &regex::Captures| {
            let e = &c[1];
            let ch = match e {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
                _ => e[1..].parse().ok().and_then(char::from_u32),
            };
            ch.map_or_else(|| c[0].to_owned(), String::from)
        })
        .into_owned()
}

/// Artistes multiples séparés par « ; » (« Band One; Singer Two; Anne-Sophie ») :
/// l'artiste principal, s'il y a plusieurs noms.
pub fn main_artist(name: &str) -> Option<String> {
    let mut parts = name.split(';').map(str::trim).filter(|p| !p.is_empty());
    let first = parts.next()?;
    name.contains(';').then(|| first.to_owned())
}

static ARTIST_SEP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*(?:&|,|;|\+|\s/\s|\bfeat\.?|\bft\.|\bfeaturing\b|\bvs\.?(?:\s|$)|\s+x\s+|\band\b|\bet\b|\bwith\b|\bavec\b)\s*").unwrap()
});
static ARTIST_ALT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*[\[(](?:aka\s+)?([^\])]+)[\])]").unwrap());

/// Noms à essayer pour retrouver un artiste dans un catalogue en ligne, du plus sûr au
/// moins sûr : le nom entier, sans sa mention entre crochets, cette mention (« 山田花子
/// [Hanako Yamada] », « Brother Max (AKA Max Kane) »), puis chaque artiste d'un nom
/// collectif (« A & B », « A vs B », « A feat. B »).
pub fn artist_names(name: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![name.trim().to_owned()];
    let bare = ARTIST_ALT.replace_all(name, "").trim().to_owned();
    out.push(bare.clone());
    out.extend(ARTIST_ALT.captures_iter(name).map(|c| c[1].trim().to_owned()));
    out.extend(ARTIST_SEP.split(&bare).map(|p| p.trim().to_owned()));
    let mut keys = std::collections::HashSet::new();
    out.retain(|n| !n.is_empty() && keys.insert(artist_key(n)));
    out
}

static NICKNAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\s+['"‘“][^'"’”]{1,24}['"’”]\s+"#).unwrap());

/// Surnom entre guillemets au milieu d'un nom retiré : « Paul 'PJ' O'Neill »
/// → « Paul O'Neill ». None s'il n'y en a pas.
pub fn without_nickname(name: &str) -> Option<String> {
    let r = NICKNAME.replace(name, " ");
    (r != name).then(|| r.into_owned())
}

// ---------- éditions d'albums ----------

/// Mentions d'édition : « (Remastered) », « [Deluxe Edition] », « - 10th Anniversary Edition »,
/// « (15 Years) », « (Édition Collector) »… Une parenthèse qui en contient une est retirée.
const EDITION: &str = r"remaster(ed)?|deluxe|anniversary|anniversaire|expanded|edition|édition|bonus|re-?issue|réédition|legacy|collector'?s?|special|spéciale|super deluxe|\d+\s*(th|st|nd|rd|e|ème)?\s*(year|years|ans)|mono|stereo|version originale|definitive";
static EDITION_BRACKET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)\s*[\(\[][^\)\]]*\b({EDITION})\b[^\)\]]*[\)\]]")).unwrap());
static EDITION_DASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(?i)\s+[-–:]\s+[^-–]*\b({EDITION})\b.*$")).unwrap());
/// Enregistrements différents de l'album studio : jamais fusionnés avec lui.
static LIVE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(live|unplugged|en concert|in concert|acoustic|acoustique|demos?|sessions?|remix(es)?|instrumentals?)\b").unwrap());

/// Titre d'album sans ses mentions d'édition : « Random Access Memories (10th Anniversary Edition) »
/// → « Random Access Memories ». Inchangé s'il n'y en a pas.
pub fn album_base(title: &str) -> String {
    let t = EDITION_BRACKET.replace_all(title, "");
    let t = EDITION_DASH.replace(&t, "");
    t.trim().to_owned()
}

/// Album enregistré en public, en acoustique, en démos, en remix…
pub fn is_live(title: &str) -> bool {
    LIVE.is_match(title)
}

// ---------- scrobbles mal formés ----------

static AUDIO_EXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.(mp3|flac|m4a|aac|ogg|oga|opus|wav|wma|alac|ape|aiff?|wv)$").unwrap());
/// Numéro de piste en tête suivi d'une ponctuation : « 03 - », « 03. », « 1-03 », « 03_ », « Track 03 - ».
static TRACK_NO_PUNCT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*(?:(?:cd|disc|disque)\s*\d+\s*[-._ ]\s*)?(?:track|piste)?\s*\d{1,3}(?:[-.]\d{1,3})?\s*(?:[-._)]|\s-)\s*").unwrap());
/// Numéro suivi d'un simple espace (« 03 Get Lucky ») : retiré seulement pour un nom de fichier,
/// sinon « 99 Luftballons » perdrait son titre.
static TRACK_NO_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\d{1,3}\s+").unwrap());
/// « 1-03 Titre » : disque 1, piste 3 (nom de fichier).
static DISC_TRACK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\d{1,2}[-.]\d{1,3}\s+").unwrap());

/// Titre en « .mp4 » : erreur d'enregistrement (vidéo scrobblée), l'écoute ne compte pas.
pub fn is_recording_error(title: &str) -> bool {
    title.trim_end().to_ascii_lowercase().ends_with(".mp4")
}

/// Titre débarrassé de ses traces de nom de fichier (extension, numéro de piste,
/// soulignés). None s'il n'y avait rien à nettoyer.
pub fn clean_filename(title: &str) -> Option<String> {
    let looks_like_file = AUDIO_EXT.is_match(title);
    let mut t = AUDIO_EXT.replace(title, "").into_owned();
    if !t.contains(' ') && t.contains('_') {
        t = t.replace('_', " ");
    }
    if looks_like_file {
        t = DISC_TRACK.replace(&t, "").into_owned();
    }
    let before = t.clone();
    t = TRACK_NO_PUNCT.replace(&t, "").into_owned();
    if looks_like_file && t == before {
        t = TRACK_NO_SPACE.replace(&t, "").into_owned();
    }
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    (t != title && !t.is_empty()).then_some(t)
}

/// « Daft Punk - Get Lucky » → (« Daft Punk », « Get Lucky »), au premier tiret entouré d'espaces.
pub fn split_artist_title(title: &str) -> Option<(String, String)> {
    for sep in [" - ", " – ", " — "] {
        if let Some((a, b)) = title.split_once(sep) {
            let (a, b) = (a.trim(), b.trim());
            if !a.is_empty() && !b.is_empty() {
                return Some((a.to_owned(), b.to_owned()));
            }
        }
    }
    None
}

/// Artistes « par défaut » des lecteurs quand le tag est absent.
pub fn is_placeholder_artist(name: &str) -> bool {
    matches!(
        fold(name).as_str(),
        "unknown" | "unknownartist" | "artisteinconnu" | "inconnu" | "variousartists" | "various" | "va" | "artistesdivers"
            | "untitledartist" | "noartist" | "sansartiste" | "unbekannterinterpret" | "desconocido"
    ) || name.trim().is_empty()
}

/// Clé d'artiste sans article initial anglais : « The Beatles » et « Beatles » se rejoignent.
pub fn artist_key_no_article(name: &str) -> String {
    let k = artist_key(name);
    k.strip_prefix("the").filter(|r| r.len() >= 3 && *r != "the").map(str::to_owned).unwrap_or(k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_join_unicode_forms_and_spellings() {
        // Japonais décomposé : rejoint la forme composée, sans tomber sur une autre lettre.
        assert_eq!(title_key("真夜中のト\u{3099}ア"), title_key("真夜中のドア"));
        assert_ne!(title_key("ト\u{3099}"), title_key("ト"));
        // Hangul en jamos.
        assert_eq!(title_key("\u{1100}\u{1161}"), title_key("가"));
        assert_eq!(title_key("Cœur"), title_key("Coeur"));
        assert_eq!(artist_key("Røyksopp"), artist_key("Royksopp"));
        assert_eq!(title_key("Straße"), title_key("Strasse"));
        assert_eq!(title_key("Jusqu'à demain (Remasterisé en 2002)"), title_key("Jusqu'à demain"));
        assert_eq!(title_key("Bella Ciao [Official Music Video]"), title_key("Bella Ciao"));
        assert_eq!(title_key("Get Lucky (2013 Edit)"), title_key("Get Lucky"));
        // « Edit » seul : un autre montage, souvent bien plus court.
        assert_ne!(title_key("Song (Studio Outtakes) [Edit]"), title_key("Song (Studio Outtakes)"));
        assert_eq!(title_key("Song (http://www.Example.net)"), title_key("Song"));
        // Une version live reste distincte.
        assert_ne!(title_key("Song (Live)"), title_key("Song"));
        assert_ne!(title_key("SUZETTE (REMIX EDIT VERSION)"), title_key("SUZETTE"));
        assert_ne!(title_key("Song - Live Edit"), title_key("Song"));
        assert_eq!(title_key("Beautiful Forest &amp; The Great Hall"), title_key("Beautiful Forest & The Great Hall"));
    }

    #[test]
    fn artist_names_for_lookup() {
        assert_eq!(artist_names("Sam Cole & Ada Reed"), ["Sam Cole & Ada Reed", "Sam Cole", "Ada Reed"]);
        assert_eq!(artist_names("山田花子 [Hanako Yamada]"), ["山田花子 [Hanako Yamada]", "山田花子", "Hanako Yamada"]);
        assert_eq!(artist_names("Brother Max (AKA Max Kane)"), ["Brother Max (AKA Max Kane)", "Brother Max", "Max Kane"]);
        assert_eq!(artist_names("Duo Nord vs Claire Sud"), ["Duo Nord vs Claire Sud", "Duo Nord", "Claire Sud"]);
        // Même clé qu'« Nova » (artist_key ignore le « feat. ») : un seul essai pour les deux.
        assert_eq!(artist_names("Nova feat. Zoé"), ["Nova feat. Zoé", "Zoé"]);
        assert_eq!(artist_names("Daft Punk"), ["Daft Punk"]);
        // « x » seul entre deux noms, pas dans un nom.
        assert_eq!(artist_names("Malcolm X"), ["Malcolm X"]);
    }

    #[test]
    fn album_editions() {
        let base = "Random Access Memories";
        assert_eq!(album_base("Random Access Memories (10th Anniversary Edition)"), base);
        assert_eq!(album_base("Random Access Memories [Deluxe]"), base);
        assert_eq!(album_base("Random Access Memories - Remastered 2023"), base);
        assert_eq!(album_base("The Blue Notebooks (15 Years)"), "The Blue Notebooks");
        assert_eq!(album_base("Giant Steps (60th Anniversary Super Deluxe Edition) [2020 Remaster]"), "Giant Steps");
        assert_eq!(album_base("Debut..."), "Debut...");
        // Pas une édition : un sous-titre reste.
        assert_eq!(album_base("Quiet Rooms (Like Lights in the Distance)"), "Quiet Rooms (Like Lights in the Distance)");
        assert!(is_live("Alive 2007 (Live)") && is_live("MTV Unplugged in New York"));
        assert!(!is_live("Random Access Memories"));
    }

    #[test]
    fn filenames() {
        assert_eq!(clean_filename("03 - Get Lucky.flac").as_deref(), Some("Get Lucky"));
        assert_eq!(clean_filename("01. Daft Punk - Get Lucky.mp3").as_deref(), Some("Daft Punk - Get Lucky"));
        assert_eq!(clean_filename("1-03 Instant Crush.m4a").as_deref(), Some("Instant Crush"));
        assert_eq!(clean_filename("Get_Lucky_(feat._Pharrell)").as_deref(), Some("Get Lucky (feat. Pharrell)"));
        assert_eq!(clean_filename("07 Giorgio by Moroder.mp3").as_deref(), Some("Giorgio by Moroder"));
        assert_eq!(clean_filename("Track 05 - Touch").as_deref(), Some("Touch"));
        // De vrais titres qui ressemblent à des numéros.
        assert_eq!(clean_filename("99 Luftballons"), None);
        assert!(is_recording_error("Get Lucky.MP4 ") && !is_recording_error("MP4 Song"));
        assert_eq!(clean_filename("1901"), None);
        assert_eq!(clean_filename("Get Lucky"), None);
    }

    #[test]
    fn split_and_placeholders() {
        assert_eq!(split_artist_title("Daft Punk - Get Lucky"), Some(("Daft Punk".into(), "Get Lucky".into())));
        assert_eq!(split_artist_title("Get Lucky"), None);
        assert!(is_placeholder_artist("Unknown Artist") && is_placeholder_artist("<unknown>") && is_placeholder_artist(""));
        assert!(!is_placeholder_artist("Daft Punk"));
        assert_eq!(artist_key_no_article("The Beatles"), artist_key_no_article("Beatles"));
        assert_eq!(artist_key_no_article("The The"), "thethe");
    }

    #[test]
    fn same_artist() {
        assert_eq!(artist_key("Björk"), artist_key("bjork"));
        assert_eq!(artist_key("AC/DC"), artist_key("ACDC"));
        assert_eq!(artist_key("Simon & Garfunkel"), artist_key("Simon and Garfunkel"));
        assert_eq!(artist_key("Mark Ronson feat. Bruno Mars"), artist_key("Mark Ronson"));
        assert_ne!(artist_key("The The"), artist_key("The"));
    }

    #[test]
    fn same_title() {
        let k = title_key("Come Together");
        assert_eq!(title_key("Come Together - Remastered 2009"), k);
        assert_eq!(title_key("Come Together (2019 Mix)"), k);
        assert_eq!(title_key("Uptown Funk (feat. Bruno Mars)"), title_key("Uptown Funk"));
        assert_eq!(title_key("Get Lucky (Radio Edit) [feat. Pharrell Williams]"), title_key("Get Lucky"));
        assert_eq!(title_key("Hey Ya!"), title_key("hey ya"));
    }

    #[test]
    fn distinct_versions_stay_distinct() {
        assert_ne!(title_key("Belle Journée - Live à l'Olympia"), title_key("Belle Journée"));
        assert_ne!(title_key("D.A.N.C.E. (Orchestral Version)"), title_key("D.A.N.C.E."));
        assert_ne!(title_key("Technologic - Digitalism Remix"), title_key("Technologic"));
    }

    #[test]
    fn non_latin_and_punctuation() {
        assert_eq!(title_key("アイドル"), "アイドル");
        assert_ne!(title_key("ド"), title_key("ト"));
        assert_eq!(artist_key("Ｙｏａｓｏｂｉ"), artist_key("YOASOBI"));
        assert_eq!(title_key("..."), "...");
        assert_ne!(title_key("?"), title_key("!"));
    }
}
