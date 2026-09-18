use tidytrail_core::{squarify, Item, Rect};

fn area(r: &Rect) -> f64 {
    r.w * r.h
}

#[test]
fn allocates_area_proportional_to_value() {
    let items = vec![
        Item {
            value: 6.0,
            data: "big",
        },
        Item {
            value: 2.0,
            data: "small",
        },
    ];
    let bounds = Rect {
        x: 0.0,
        y: 0.0,
        w: 8.0,
        h: 4.0,
    };
    let placed = squarify(&items, bounds);

    let total_area = area(&bounds);
    let big = placed.iter().find(|(d, _)| *d == "big").unwrap();
    let small = placed.iter().find(|(d, _)| *d == "small").unwrap();

    assert!((area(&big.1) - total_area * 0.75).abs() < 1e-6);
    assert!((area(&small.1) - total_area * 0.25).abs() < 1e-6);
}

#[test]
fn rectangles_do_not_overflow_the_bounds() {
    let items: Vec<Item<usize>> = (0..12)
        .map(|i| Item {
            value: (i + 1) as f64,
            data: i,
        })
        .collect();
    let bounds = Rect {
        x: 10.0,
        y: 20.0,
        w: 300.0,
        h: 150.0,
    };
    let placed = squarify(&items, bounds);

    assert_eq!(placed.len(), items.len());
    for (_, r) in &placed {
        assert!(r.x >= bounds.x - 1e-6);
        assert!(r.y >= bounds.y - 1e-6);
        assert!(r.x + r.w <= bounds.x + bounds.w + 1e-6);
        assert!(r.y + r.h <= bounds.y + bounds.h + 1e-6);
    }
}

#[test]
fn zero_and_negative_values_are_dropped() {
    let items = vec![
        Item {
            value: 5.0,
            data: "keep",
        },
        Item {
            value: 0.0,
            data: "drop-zero",
        },
        Item {
            value: -3.0,
            data: "drop-negative",
        },
    ];
    let placed = squarify(
        &items,
        Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        },
    );

    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].0, "keep");
}

#[test]
fn empty_input_produces_no_rectangles() {
    let items: Vec<Item<()>> = Vec::new();
    let placed = squarify(
        &items,
        Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        },
    );
    assert!(placed.is_empty());
}
