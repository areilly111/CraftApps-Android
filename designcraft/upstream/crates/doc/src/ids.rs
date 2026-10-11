//! Stable identifiers. All ids are unique within a document (one counter).

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
        #[serde(transparent)]
        pub struct $name(pub u64);
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    };
}

id_type!(ItemId, "i");
id_type!(StoryId, "s");
id_type!(SpreadId, "sp");
id_type!(PageId, "p");
id_type!(LayerId, "l");
id_type!(AssetId, "a");
