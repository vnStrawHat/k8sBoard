use gpui_kit::{hsla, rgba};

use super::*;
use crate::topology_fixtures::{Fixture, Ref, crashing_pod, ingress, pod, pod_with};
use crate::topology_graph::GroupBy;
use crate::topology_layout::layout;

fn style() -> SvgStyle {
    SvgStyle {
        background: "#010203".to_owned(),
        border: "#040506".to_owned(),
        text: "#070809".to_owned(),
        muted: "#0a0b0c".to_owned(),
        badge: "#0d0e0f".to_owned(),
        accent: "#101112".to_owned(),
        warn: "#131415".to_owned(),
        bad: "#161718".to_owned(),
        font_family: "Test Mono".to_owned(),
    }
}

fn svg_of(fixture: &Fixture, title: &str) -> (TopologyGraph, String) {
    let graph = fixture.graph();
    let arranged = layout(&graph, GroupBy::Components, 1., &Default::default(), None);
    let svg = topology_svg(&graph, &arranged, title, &style());
    (graph, svg)
}

#[test]
fn svg_has_a_node_per_graph_node() {
    let fixture = Fixture::default()
        .with_service("web", &["app=web"])
        .with_pods([
            pod("web-1", &["app=web"], None),
            crashing_pod("web-2", &["app=web"], None),
        ])
        .with_deployment("api", 1, 1);
    let (graph, svg) = svg_of(&fixture, "t");
    assert_eq!(svg.matches("<g opacity=").count(), graph.nodes.len());
    assert!(svg.starts_with("<svg "));
    assert!(svg.trim_end().ends_with("</svg>"));
}

#[test]
fn svg_escapes_names() {
    let fixture = Fixture::default().with_deployment("a&b<c>\"d'", 1, 1);
    let (_, svg) = svg_of(&fixture, "x & <y>");
    assert!(svg.contains("a&amp;b&lt;c&gt;&quot;d&apos;"));
    assert!(svg.contains("x &amp; &lt;y&gt;"));
    assert!(!svg.contains("a&b<c>"));
}

#[test]
fn svg_dash_per_relation() {
    let owns = Fixture::default()
        .with_deployment("api", 1, 1)
        .with_replica_set("api-1", Some("api"), 1, 1);
    let (_, svg) = svg_of(&owns, "t");
    assert!(svg.contains("<path d=\"M"));
    assert!(!svg.contains("stroke-dasharray"));
    let routes = Fixture::default()
        .with_service("web", &["app=web"])
        .with_pod(pod("web-1", &["app=web"], None));
    assert!(svg_of(&routes, "t").1.contains("stroke-dasharray=\"5 4\""));
    let mounts = Fixture::default()
        .with_config_map("settings")
        .with_pod(pod_with(
            pod("tool", &[], None),
            &[Ref::EnvConfigMap("settings")],
        ));
    assert!(svg_of(&mounts, "t").1.contains("stroke-dasharray=\"2 3\""));
}

#[test]
fn svg_uses_style_colors() {
    let fixture = Fixture::default()
        .with_service("web", &["app=web"])
        .with_pod(crashing_pod("web-1", &["app=web"], None))
        .with_ingress(ingress("shop", &[("/", "gone")], None, None));
    let (_, svg) = svg_of(&fixture, "t");
    let style = style();
    // The background, the title text, a routes edge, and a Bad card and ghost edge.
    for color in [
        &style.background,
        &style.text,
        &style.accent,
        &style.bad,
        &style.muted,
    ] {
        assert!(svg.contains(color.as_str()), "{color}");
    }
    assert!(svg.contains("font-family=\"Test Mono, monospace\""));
}

#[test]
fn hex_formats_rrggbb() {
    assert_eq!(hex(hsla(0., 0., 1., 1.)), "#ffffff");
    assert_eq!(hex(hsla(0., 0., 0., 1.)), "#000000");
    assert_eq!(hex(hsla(0., 1., 0.5, 1.)), "#ff0000");
    // The alpha is dropped.
    assert_eq!(hex(hsla(0., 0., 1., 0.25)), "#ffffff");
    assert_eq!(hex(rgba(0x336699ff).into()), "#336699");
}

#[test]
fn svg_title_names_namespace() {
    let title =
        "Topology \u{b7} ctx \u{b7} ns: shop \u{b7} 2 resources \u{b7} 2024-05-01T10:00:00Z";
    let (_, svg) = svg_of(&Fixture::default().with_deployment("api", 1, 1), title);
    assert!(svg.contains("ns: shop"));
    assert!(svg.contains("2024-05-01T10:00:00Z"));
}

#[test]
fn fit_chars_cuts_with_ellipsis() {
    assert_eq!(fit_chars(100., 10.), 16);
    assert_eq!(fit_chars(0., 10.), 0);
    assert_eq!(fitted("short", 10), "short");
    assert_eq!(fitted("abcdefghij", 5), "abcd\u{2026}");
    assert_eq!(fitted("abcde", 5), "abcde");
}

#[test]
fn render_png_writes_png_signature() {
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\">\
               <rect width=\"10\" height=\"10\" fill=\"#ff0000\"/></svg>";
    let png = render_png(svg).expect("a text-free SVG renders");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
}

#[test]
fn render_png_rejects_a_broken_svg() {
    let error = render_png("<svg").expect_err("not an SVG");
    assert!(matches!(error, ExportError::Render(_)));
}

#[test]
fn export_scale_caps_longest_side_at_4096() {
    assert_eq!(export_scale(1000., 500.), EXPORT_SCALE);
    assert_eq!(export_scale(8192., 100.), 0.5);
    assert_eq!(export_scale(100., 16384.), 0.25);
    assert_eq!(export_scale(0., 0.), EXPORT_SCALE);
}

#[test]
fn svg_path_extension_is_case_insensitive() {
    assert!(is_svg_path(Path::new("graph.svg")));
    assert!(is_svg_path(Path::new("graph.SVG")));
    assert!(!is_svg_path(Path::new("graph.png")));
    assert!(!is_svg_path(Path::new("svg")));
}

#[test]
fn export_error_messages_name_the_cause() {
    assert_eq!(
        ExportError::Render("bad tag".to_owned()).to_string(),
        "could not render the topology: bad tag"
    );
    assert_eq!(ExportError::EmptyImage.to_string(), "the image is empty");
    assert_eq!(
        ExportError::Encode("disk".to_owned()).to_string(),
        "could not encode the image: disk"
    );
}

#[test]
fn write_export_picks_the_format_from_the_extension() {
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"4\"/>";
    let directory = std::env::temp_dir();
    let as_svg = directory.join("k8sboard-topology-export-test.SVG");
    write_export(
        &as_svg,
        TopologyExport {
            svg: svg.to_owned(),
        },
    )
    .expect("writes the SVG");
    assert_eq!(std::fs::read_to_string(&as_svg).expect("reads"), svg);
    let as_png = directory.join("k8sboard-topology-export-test.png");
    write_export(
        &as_png,
        TopologyExport {
            svg: svg.to_owned(),
        },
    )
    .expect("writes the PNG");
    assert_eq!(&std::fs::read(&as_png).expect("reads")[1..4], b"PNG");
    let _ = std::fs::remove_file(as_svg);
    let _ = std::fs::remove_file(as_png);
}
