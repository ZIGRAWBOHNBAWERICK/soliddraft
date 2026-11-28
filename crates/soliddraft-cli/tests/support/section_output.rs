use soliddraft_kernel_api::Point2;

pub fn svg_segments(svg: &str, layer: &str, paper_height: f64) -> Vec<[Point2; 2]> {
    let start = format!(r#"<g id="{layer}""#);
    let (group, _) = svg.split_once(&start).expect("drawing layer").1.split_once("</g>").unwrap();
    let mut segments = Vec::new();
    for path in group.split(r#"<path d=""#).skip(1) {
        let mut tokens = path.split('"').next().unwrap().split_whitespace();
        let mut last = None;
        let mut start = None;
        while let Some(head) = tokens.next() {
            let (cmd, x) = head.split_at(1);
            let mut point = |x: &str| {
                Point2::new(x.parse().unwrap(), paper_height - tokens.next().unwrap().parse::<f64>().unwrap())
            };
            match cmd {
                "M" => {
                    let p = point(x);
                    start = Some(p);
                    last = Some(p);
                }
                "L" => {
                    let p = point(x);
                    if let Some(from) = last.replace(p) {
                        segments.push([from, p]);
                    }
                }
                "Q" | "C" => {
                    let _control = point(x);
                    if cmd == "C" {
                        tokens.next();
                        tokens.next();
                    }
                    let x = tokens.next().unwrap().parse::<f64>().unwrap();
                    let y = tokens.next().unwrap().parse::<f64>().unwrap();
                    last = Some(Point2::new(x, paper_height - y));
                }
                "Z" => {
                    if let (Some(a), Some(b)) = (last, start) {
                        segments.push([a, b]);
                    }
                    last = start;
                }
                _ => panic!("unexpected SVG path command {cmd}"),
            }
        }
    }
    segments
}

pub fn hatch_loops(bytes: &[u8]) -> Vec<Vec<(bool, Vec<Point2>)>> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines();
    let groups: Vec<_> = std::iter::from_fn(|| {
        let code = lines.next()?.trim().parse::<u16>().expect("DXF group code");
        Some((code, lines.next().expect("DXF group value").trim()))
    })
    .collect();
    groups
        .chunk_by(|_, next| next.0 != 0)
        .filter(|entity| entity.first() == Some(&(0, "HATCH")))
        .map(|entity| {
            let start = entity.iter().position(|(code, _)| *code == 92).expect("DXF hatch boundary");
            entity[start..]
                .chunk_by(|_, next| next.0 != 92)
                .map(|groups| {
                    let (92, flag) = groups[0] else { panic!("DXF loop flag") };
                    let external = flag.parse::<u32>().unwrap() & 1 != 0;
                    let ring = groups.iter().position(|(code, _)| *code == 97).expect("end of DXF boundary");
                    let points = groups[..ring]
                        .windows(2)
                        .filter_map(|pair| {
                            let [(10, x), (20, y)] = pair else { return None };
                            Some(Point2::new(x.parse().unwrap(), y.parse().unwrap()))
                        })
                        .collect();
                    (external, points)
                })
                .collect()
        })
        .collect()
}
