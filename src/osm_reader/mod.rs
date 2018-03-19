extern crate mimir;
extern crate osmpbfreader;
extern crate failure;

use std::fs::File;
use std::path::Path;
use Error;

pub mod osm_utils;
pub mod admin;
pub mod poi;
pub mod street;

pub type OsmPbfReader = osmpbfreader::OsmPbfReader<File>;


pub fn make_osm_reader(path: &Path) -> Result<OsmPbfReader, Error>  {
    Ok(osmpbfreader::OsmPbfReader::new(File::open(&path)?))
}
