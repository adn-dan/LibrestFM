//! Cœur de LibrestFM : import et fusion des historiques d'écoute, statistiques,
//! enrichissement des titres (audio features). Aucune dépendance à Tauri, pour
//! pouvoir être testé seul et réutilisé (CLI, autre interface).

// Les lignes de requêtes sont lues en tuples (id, nom, compte…) à l'endroit où elles servent :
// les nommer par un alias de type à chaque fois n'aiderait pas la lecture.
#![allow(clippy::type_complexity)]

pub mod accounts;
pub mod aliases;
pub mod backup;
pub mod credits;
pub mod db;
pub mod discover;
pub mod enrich;
pub mod export;
pub mod images;
pub mod info;
pub mod import;
pub mod inbox;
pub mod lyrics;
pub mod mixes;
pub mod normalize;
pub mod stats;

pub use db::Library;

/// Taille maximale d'une réponse JSON d'un service (les plus grosses, une page de 1 000
/// écoutes Libre.fm ou un catalogue, font quelques Mo) : une réponse anormale ne remplit pas
/// la mémoire.
const MAX_JSON: u64 = 32 * 1024 * 1024;

/// Corps JSON d'une réponse, lu dans la limite de `MAX_JSON` (remplace `into_json`).
pub(crate) fn json_body<T: serde::de::DeserializeOwned>(r: ureq::Response) -> std::io::Result<T> {
    use std::io::Read;
    serde_json::from_reader(r.into_reader().take(MAX_JSON)).map_err(std::io::Error::other)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("base de données : {0}")]
    Db(#[from] rusqlite::Error),
    #[error("fichier : {0}")]
    Io(#[from] std::io::Error),
    #[error("format non reconnu : {0}")]
    UnknownFormat(String),
    #[error("réseau : {0}")]
    Http(String),
    #[error("introuvable : {0}")]
    NotFound(String),
    #[error("annulé")]
    Cancelled,
    /// Contenu refusé ; le code est traduit par l'interface (« not_sqlite »…).
    #[error("invalide : {0}")]
    Invalid(&'static str),
}

impl Error {
    /// Code stable, traduit par l'interface : « network », « not_found », « invalid.not_sqlite »…
    pub fn code(&self) -> String {
        match self {
            Error::Db(_) => "db".into(),
            Error::Io(_) => "io".into(),
            Error::UnknownFormat(_) => "format".into(),
            Error::Http(_) => "network".into(),
            Error::NotFound(_) => "not_found".into(),
            Error::Cancelled => "cancelled".into(),
            Error::Invalid(c) => format!("invalid.{c}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
