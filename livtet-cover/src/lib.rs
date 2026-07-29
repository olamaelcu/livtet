pub mod encode;
pub mod error;
pub mod fetcher;
pub mod storage;

pub use encode::{CoverMetadata, EncodeError, encode_cover};
pub use error::{CoverError, CoverResult};
pub use fetcher::{CacheKey, CoverFetcher, FetchError, FetchedCover};
pub use storage::{CachedCover, CoverStorage};
