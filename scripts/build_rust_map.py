"""
This script is used for building a routee-compass map from a pre-downloaded set
of parquet files that represent the whole US road network from the trolley
tomtom_multinet_current schema. 

This script has a companion file `build_rust_map.sh` for running this script
on an eagle node.
"""
from pathlib import Path
from typing import Dict, List, Tuple
import pandas as pd
import pickle
import time
import os
import glob
import logging
import argparse
import sqlalchemy as sql

from nrel.routee.compass import Graph, Link, Node, RustMap, extract_largest_scc

from shapely.geometry import LineString
from shapely import from_wkb
from tqdm import tqdm

logging.basicConfig(level=logging.INFO)

log = logging.getLogger(__name__)

USER = os.environ.get("TROLLEY_USERNAME")
PASSWORD = os.environ.get("TROLLEY_PASSWORD")

parser = argparse.ArgumentParser()
parser.add_argument("working_dir", type=str, default="", help="working directory")

args = parser.parse_args()

WORKING_DIR = Path(args.working_dir)

RESTRICTION_FILE = WORKING_DIR / "restrictions.pickle"

LATLON = "epsg:4326"
WEB_MERCATOR = "epsg:3857"

# also referred to as the 'positive' direction in TomTom
FROM_TO_DIRECTION = 2

# also referred to as the 'negative' direction in TomTom
TO_FROM_DIRECTION = 3

DEFAULT_SPEED_KPH = 40

MAX_ROUTING_CLASS = 4


def build_speed(t, direction) -> int:
    # optimisitically return free flow
    if not pd.isna(t.free_flow_speed):
        return int(t.free_flow_speed)

    if direction == TO_FROM_DIRECTION:
        if not pd.isna(t.speed_average_neg):
            return int(t.speed_average_neg)
        return DEFAULT_SPEED_KPH
    elif direction == FROM_TO_DIRECTION:
        if not pd.isna(t.speed_average_pos):
            return int(t.speed_average_pos)
        return DEFAULT_SPEED_KPH
    else:
        raise ValueError("Bad direction value")


def build_link(
    t, direction, node_id_mapping, node_id_counter
) -> Tuple[Link, Node, Node, Dict[str, int], int]:
    if t.junction_id_to in node_id_mapping:
        junction_id_to_id = node_id_mapping[t.junction_id_to]
    else:
        junction_id_to_id = node_id_counter
        node_id_mapping[t.junction_id_to] = junction_id_to_id
        node_id_counter += 1

    if t.junction_id_from in node_id_mapping:
        junction_id_from_id = node_id_mapping[t.junction_id_from]
    else:
        junction_id_from_id = node_id_counter
        node_id_mapping[t.junction_id_from] = junction_id_from_id
        node_id_counter += 1

    speed_kph = build_speed(t, direction)
    if direction == TO_FROM_DIRECTION:
        geom = LineString(reversed(t.geom.coords))
        start_point = geom.coords[0]
        end_point = geom.coords[-1]
        grade = -t.mean_gradient_dec
        start_node = Node(junction_id_to_id, int(start_point[0]), int(start_point[1]))
        end_node = Node(junction_id_from_id, int(end_point[0]), int(end_point[1]))
    elif direction == FROM_TO_DIRECTION:
        geom = t.geom
        start_point = geom.coords[0]
        end_point = geom.coords[-1]
        grade = t.mean_gradient_dec
        start_node = Node(junction_id_from_id, int(start_point[0]), int(start_point[1]))
        end_node = Node(junction_id_to_id, int(end_point[0]), int(end_point[1]))
    else:
        raise ValueError("Bad direction value")

    if weight_restrictions is not None:
        wr = weight_restrictions.get(t.netw_id)
        if wr is None:
            weight_restriction = None
        else:
            if 1 in wr:
                # restriction is bidirectional
                weight_restriction = int(wr[1] * 2000)  # tons to lbs
            elif direction in wr:
                # restriction is unidirectional
                weight_restriction = int(wr[direction] * 2000)  # tons to lbs
            else:
                weight_restriction = None
    else:
        weight_restriction = None

    if height_restrictions is not None:
        hr = height_restrictions.get(t.netw_id)
        if hr is None:
            height_restriction = None
        else:
            if 1 in hr:
                # restriction is bidirectional
                height_restriction = int(hr[1])
            elif direction in hr:
                # restriction is unidirectional
                height_restriction = int(hr[direction])
            else:
                height_restriction = None
    else:
        height_restriction = None

    if width_restrictions is not None:
        wdr = width_restrictions.get(t.netw_id)
        if wdr is None:
            width_restriction = None
        else:
            if 1 in wdr:
                # restriction is bidirectional
                width_restriction = int(wdr[1])
            elif direction in wdr:
                # restriction is unidirectional
                width_restriction = int(wdr[direction])
            else:
                width_restriction = None
    else:
        width_restriction = None

    if length_restrictions is not None:
        lr = length_restrictions.get(t.netw_id)
        if lr is None:
            length_restriction = None
        else:
            if 1 in lr:
                # restriction is bidirectional
                length_restriction = int(lr[1])
            elif direction in lr:
                # restriction is unidirectional
                length_restriction = int(lr[direction])
            else:
                length_restriction = None
    else:
        length_restriction = None

    if pd.isna(grade):
        grade_milli = 0
    else:
        grade_milli = int(grade * 1000)

    distance_cm = t.centimeters

    week_profile_ids = [
        profile_id_integer_mapping.get(t.monday_profile_id),
        profile_id_integer_mapping.get(t.tuesday_profile_id),
        profile_id_integer_mapping.get(t.wednesday_profile_id),
        profile_id_integer_mapping.get(t.thursday_profile_id),
        profile_id_integer_mapping.get(t.friday_profile_id),
        profile_id_integer_mapping.get(t.saturday_profile_id),
        profile_id_integer_mapping.get(t.sunday_profile_id),
    ]

    link = Link(
        start_node.id,
        end_node.id,
        speed_kph,
        distance_cm,
        grade_milli,
        week_profile_ids,
        weight_restriction,
        height_restriction,
        width_restriction,
        length_restriction,
    )

    return link, start_node, end_node, node_id_mapping, node_id_counter


def links_from_df(
    df, node_id_mapping, node_id_counter
) -> Tuple[List[Link], List[Node], Dict[str, int], int]:
    df = df[df.routing_class <= MAX_ROUTING_CLASS]
    df["link_direction"] = df.link_direction.fillna(9).astype(int)
    df["geom"] = df.geom.apply(lambda g: from_wkb(g))

    oneway_ft = df[df.link_direction == FROM_TO_DIRECTION]
    oneway_tf = df[df.link_direction == TO_FROM_DIRECTION]
    twoway = df[df.link_direction.isin([1, 9])]

    assert len(oneway_ft) + len(oneway_tf) + len(twoway) == len(df)

    links = []
    nodes = []
    two_way_tf_links = []
    for t in twoway.itertuples():
        link, start_node, end_node, node_id_mapping, node_id_counter = build_link(
            t, TO_FROM_DIRECTION, node_id_mapping, node_id_counter
        )
        nodes.append(start_node)
        nodes.append(end_node)
        two_way_tf_links.append(link)
    links.extend(two_way_tf_links)

    two_way_ft_links = []
    for t in twoway.itertuples():
        link, start_node, end_node, node_id_mapping, node_id_counter = build_link(
            t, FROM_TO_DIRECTION, node_id_mapping, node_id_counter
        )
        nodes.append(start_node)
        nodes.append(end_node)
        two_way_ft_links.append(link)
    links.extend(two_way_ft_links)

    oneway_ft_links = []
    for t in oneway_ft.itertuples():
        link, start_node, end_node, node_id_mapping, node_id_counter = build_link(
            t, FROM_TO_DIRECTION, node_id_mapping, node_id_counter
        )
        nodes.append(start_node)
        nodes.append(end_node)
        oneway_ft_links.append(link)
    links.extend(oneway_ft_links)

    oneway_tf_links = []
    for t in oneway_tf.itertuples():
        link, start_node, end_node, node_id_mapping, node_id_counter = build_link(
            t, TO_FROM_DIRECTION, node_id_mapping, node_id_counter
        )
        nodes.append(start_node)
        nodes.append(end_node)
        oneway_tf_links.append(link)
    links.extend(oneway_tf_links)

    return links, nodes, node_id_mapping, node_id_counter


if __name__ == "__main__":
    engine = sql.create_engine(f"postgresql://{USER}:{PASSWORD}@trolley.nrel.gov:5432/master")

    profile_mapping_file = WORKING_DIR / "profile_id_mapping.csv"
    if USER is None and PASSWORD is None:
        if not profile_mapping_file.exists():
            raise ValueError(
                "profile_id_mapping.csv not found in working directory. "
                "Please provide a username and password to connect to trolley."
            )
        df = pd.read_csv(WORKING_DIR / "profile_id_mapping.csv").set_index("profile_id")
        profile_id_integer_mapping = {}
        for pid, r in df.iterrows():
            profile_id_integer_mapping[pid] = int(r.profile_id_integer)
    else:
        log.info("getting speed by time of day info from trolley..")

        q = """
        select profile_id, speed_per_time_slot_id
        from tomtom_multinet_current.mnr_profile2speed_per_time_slot
        """

        sdf = pd.read_sql(q, engine)
        sdf = sdf.astype(str)

        tq = """
        select *
        from tomtom_multinet_current.mnr_speed_per_time_slot
        """

        tdf = pd.read_sql(tq, engine)
        tdf["speed_per_time_slot_id"] = tdf.speed_per_time_slot_id.astype(str)

        df = (
            sdf.set_index("speed_per_time_slot_id")
            .join(tdf.set_index("speed_per_time_slot_id"))
            .reset_index()
            .drop(columns="speed_per_time_slot_id")
        )

        profile_id_integer_mapping = {}
        for i, pid in enumerate(df.profile_id.unique()):
            profile_id_integer_mapping[pid] = i

        df["profile_id_integer"] = df.profile_id.apply(lambda pid: profile_id_integer_mapping[pid])

        df.to_csv(profile_mapping_file, index=False)

    log.info("loading restrictions..")
    with open(RESTRICTION_FILE, "rb") as rf:
        restrictions = pickle.load(rf)
    weight_restrictions = restrictions["weight"]
    height_restrictions = restrictions["height"]
    width_restrictions = restrictions["width"]
    length_restrictions = restrictions["length"]

    # NOTE: This is predicated on first running the download_us_network.py script
    log.info("loading links from file")
    files = glob.glob(str(WORKING_DIR / "*.parquet"))
    start_time = time.time()
    node_id_mapping: Dict[str, int] = {}
    node_id_counter = 0
    all_links = []
    all_nodes = []
    for nfile in tqdm(files):
        log.info(f"working on file: {nfile}")
        chunk = pd.read_parquet(nfile)
        more_links, nodes, node_id_mapping, node_id_counter = links_from_df(
            chunk, node_id_mapping, node_id_counter
        )
        all_links.extend(more_links)
        all_nodes.extend(nodes)
    log.info(f"found {len(all_links)} links")
    elsapsed_time = time.time() - start_time

    node_map_outfile = WORKING_DIR / "node-id-mapping.pickle"
    with node_map_outfile.open("wb") as nmf:
        pickle.dump(node_id_mapping, nmf)

    log.info("getting stop sign info from trolley..")
    stop_sign_query = """
    select 
    nw.junction_id_from,
    nw.junction_id_to,
    nw.routing_class,
    rf.validity_direction
    from tomtom_multinet_current.mnr_road_furniture as rf
    left join tomtom_multinet_current.mnr_netw_route_link as nw
    on rf.netw_id = nw.feat_id
    where rf.feat_type = 7403
    """

    stop_df = pd.read_sql(stop_sign_query, engine)
    stop_df = stop_df[stop_df.routing_class <= MAX_ROUTING_CLASS].astype(
        {
            "validity_direction": int,
            "junction_id_to": str,
            "junction_id_from": str,
        }
    )
    stop_sign_nodes = stop_df.apply(
        lambda row: row.junction_id_to if row.validity_direction == 2 else row.junction_id_from,
        axis=1,
    )
    stop_sign_nodes = stop_sign_nodes.rename("junction_id")
    stop_sign_nodes = stop_sign_nodes.apply(lambda nid: node_id_mapping[nid])

    stop_sign_file = WORKING_DIR / "stop_signs.csv"
    stop_sign_nodes.to_csv(stop_sign_file)

    log.info("getting traffic light info from trolley..")
    tl_q = """
    select junction_id from tomtom_multinet_current.mnr_traffic_light
    """

    tl_df = pd.read_sql(tl_q, engine)
    tl_df = tl_df.astype({"junction_id": str})
    tl_nodes = tl_df.junction_id.apply(lambda nid: node_id_mapping[nid])

    tl_outfile = WORKING_DIR / "traffic_lights.csv"
    tl_nodes.to_csv(tl_outfile)

    del node_id_mapping

    log.info("building graph..")
    start_time = time.time()
    graph = Graph()
    graph.add_nodes_bulk(all_nodes)
    graph.add_links_bulk(all_links)
    elsapsed_time = time.time() - start_time
    log.info(f"graph building took {elsapsed_time} seconds")
    number_of_links = graph.number_of_links()
    log.info(f"graph has {number_of_links} links")

    del all_links
    del all_nodes

    log.info("extracting largest scc..")
    start_time = time.time()
    graph = extract_largest_scc(graph)
    elsapsed_time = time.time() - start_time
    log.info(f"largest scc took {elsapsed_time} seconds")
    number_of_links = graph.number_of_links()
    log.info(f"graph has {number_of_links} links")

    log.info("building rust map from graph..")
    start_time = time.time()
    rust_map = RustMap(graph)
    elsapsed_time = time.time() - start_time
    log.info(f"rust map took {elsapsed_time} seconds")

    log.info("saving rust map..")
    start_time = time.time()
    rust_map.to_file(str(WORKING_DIR / "rust_map.bin"))
    elsapsed_time = time.time() - start_time
    log.info(f"saving rust map took {elsapsed_time} seconds")
