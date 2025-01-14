// Copyright © 2016, Canal TP and/or its affiliates. All rights reserved.
//
// This file is part of Navitia,
//     the software to build cool stuff with public transport.
//
// Hope you'll enjoy and contribute to this project,
//     powered by Canal TP (www.canaltp.fr).
// Help us simplify mobility and open public transport:
//     a non ending quest to the responsive locomotion way of traveling!
//
// LICENCE: This program is free software; you can redistribute it
// and/or modify it under the terms of the GNU Affero General Public
// License as published by the Free Software Foundation, either
// version 3 of the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful, but
// WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU
// Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public
// License along with this program. If not, see
// <http://www.gnu.org/licenses/>.
//
// Stay tuned using
// twitter @navitia
// IRC #navitia on freenode
// https://groups.google.com/d/forum/navitia
// www.navitia.io
use super::osm_utils::get_way_coord;
use super::OsmPbfReader;
use crate::admin_geofinder::AdminGeoFinder;
use crate::{labels, utils, Error};
use failure::ResultExt;
use slog_scope::info;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Deref;
use std::sync::Arc;

pub type AdminSet = BTreeSet<Arc<mimir::Admin>>;
pub type NameAdminMap = BTreeMap<StreetKey, Vec<osmpbfreader::OsmId>>;
pub type StreetsVec = Vec<mimir::Street>;
pub type StreetWithRelationSet = BTreeSet<osmpbfreader::OsmId>;

#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct StreetKey {
    pub name: String,
    pub admins: AdminSet,
}

pub fn streets(
    pbf: &mut OsmPbfReader,
    admins_geofinder: &AdminGeoFinder,
) -> Result<StreetsVec, Error> {
    fn is_valid_obj(obj: &osmpbfreader::OsmObj) -> bool {
        match *obj {
            osmpbfreader::OsmObj::Way(ref way) => {
                way.tags.get("highway").map_or(false, |v| !v.is_empty())
                    && way.tags.get("name").map_or(false, |v| !v.is_empty())
            }
            osmpbfreader::OsmObj::Relation(ref rel) => rel
                .tags
                .get("type")
                .map_or(false, |v| v == "associatedStreet"),
            _ => false,
        }
    }
    info!("reading pbf...");
    let objs_map = pbf
        .get_objs_and_deps(is_valid_obj)
        .context("Error occurred when reading pbf")?;
    info!("reading pbf done.");
    let mut street_rel: StreetWithRelationSet = BTreeSet::new();
    let mut street_list: StreetsVec = vec![];
    // Sometimes, streets can be divided into several "way"s that still have the same street name.
    // The reason why a street is divided may be that a part of the street become
    // a bridge/tunnel/etc. In this case, a "relation" tagged with (type = associatedStreet) is used
    // to group all these "way"s. In order not to have duplicates in autocompletion, we should tag
    // the osm ways in the relation not to index them twice.

    for rel in objs_map.iter().filter_map(|(_, obj)| obj.relation()) {
        let way_name = rel.tags.get("name");
        rel.refs
            .iter()
            .filter(|ref_obj| ref_obj.member.is_way() && ref_obj.role == "street")
            .filter_map(|ref_obj| {
                let way = objs_map.get(&ref_obj.member)?.way()?;
                let way_name = way_name.or_else(|| way.tags.get("name"))?;
                let admins = get_street_admin(admins_geofinder, &objs_map, way);
                let country_codes = utils::find_country_codes(admins.iter().map(|a| a.deref()));
                let street_label = labels::format_street_label(
                    &way_name,
                    admins.iter().map(|a| a.deref()),
                    &country_codes,
                );
                let coord = get_way_coord(&objs_map, way);
                Some(mimir::Street {
                    id: format!("street:osm:relation:{}", rel.id.0.to_string()),
                    name: way_name.to_string(),
                    label: street_label,
                    weight: 0.,
                    zip_codes: utils::get_zip_codes_from_admins(&admins),
                    administrative_regions: admins,
                    coord: get_way_coord(&objs_map, way),
                    approx_coord: Some(coord.into()),
                    distance: None,
                    country_codes,
                    context: None,
                })
            })
            .next()
            .map(|street| street_list.push(street));

        // Add osmid of all the relation members in the set
        // We don't create any street for all the osmid present in street_rel
        for ref_obj in &rel.refs {
            if ref_obj.member.is_way() {
                street_rel.insert(ref_obj.member);
            }
        }
    }

    // we merge all the ways with a key = way_name + admin list of level(=city_level)
    // we use a map NameAdminMap <key, value> to manage the merging of ways
    let keys_ids = objs_map
        .iter()
        .filter(|(osmid, _)| !street_rel.contains(osmid))
        .filter_map(|(osmid, obj)| {
            let way = obj.way()?;
            let name = way.tags.get("name")?.to_string();
            let admins = get_street_admin(admins_geofinder, &objs_map, way)
                .into_iter()
                .filter(|admin| admin.is_city())
                .collect();
            Some((StreetKey { name, admins }, osmid))
        });
    let mut name_admin_map = NameAdminMap::default();
    for (key, id) in keys_ids {
        name_admin_map.entry(key).or_insert(vec![]).push(*id);
    }

    // Create a street for each way with osmid present in objs_map
    let streets = name_admin_map.values().filter_map(|way_ids| {
        let min_id = way_ids.iter().min()?;
        let way = objs_map.get(&min_id)?.way()?;
        let name = way.tags.get("name")?.to_string();
        let admins = get_street_admin(admins_geofinder, &objs_map, way);

        let country_codes = utils::find_country_codes(admins.iter().map(|a| a.deref()));
        let street_label =
            labels::format_street_label(&name, admins.iter().map(|a| a.deref()), &country_codes);
        let coord = get_way_coord(&objs_map, way);
        Some(mimir::Street {
            id: format!("street:osm:way:{}", way.id.0.to_string()),
            label: street_label,
            name,
            weight: 0.,
            zip_codes: utils::get_zip_codes_from_admins(&admins),
            administrative_regions: admins,
            coord: get_way_coord(&objs_map, way),
            approx_coord: Some(coord.into()),
            distance: None,
            country_codes,
            context: None,
        })
    });
    street_list.extend(streets);

    Ok(street_list)
}

fn get_street_admin(
    admins_geofinder: &AdminGeoFinder,
    obj_map: &BTreeMap<osmpbfreader::OsmId, osmpbfreader::OsmObj>,
    way: &osmpbfreader::objects::Way,
) -> Vec<Arc<mimir::Admin>> {
    /*
        To avoid corner cases where the ends of the way are near
        administrative boundaries, the geofinder is called
        on a middle node.
    */
    let nb_nodes = way.nodes.len();
    way.nodes
        .iter()
        .skip(nb_nodes / 2)
        .filter_map(|node_id| obj_map.get(&(*node_id).into()))
        .filter_map(|node_obj| node_obj.node())
        .map(|node| geo_types::Coordinate {
            x: node.lon(),
            y: node.lat(),
        })
        .next()
        .map_or(vec![], |c| admins_geofinder.get(&c))
}

pub fn compute_street_weight(streets: &mut StreetsVec) {
    for st in streets {
        for admin in &mut st.administrative_regions {
            if admin.is_city() {
                st.weight = admin.weight;
                break;
            }
        }
    }
}
