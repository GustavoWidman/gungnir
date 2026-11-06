use super::*;

#[test]
fn dimensions_parse_by_name_and_reject_others() {
    for dimension in Dimension::ALL {
        assert_eq!(dimension.as_str().parse::<Dimension>().unwrap(), dimension);
    }
    let err = "runtime".parse::<Dimension>().unwrap_err();
    assert!(err.to_string().contains("pass, tokens"), "{err}");
}

#[test]
fn weights_default_to_one_and_refuse_what_cannot_weigh() {
    let weights = Weights::new([(Dimension::Speed, 0.0), (Dimension::Pass, 2.5)]).unwrap();
    assert_eq!(
        Dimension::ALL.map(|dimension| weights.of(dimension)),
        [2.5, 1.0, 1.0, 0.0, 1.0]
    );
    for (set, message) in [
        (vec![(Dimension::Time, -1.0)], "at least 0"),
        (vec![(Dimension::Time, f64::NAN)], "at least 0"),
        (
            vec![(Dimension::Time, 1.0), (Dimension::Time, 2.0)],
            "weighed twice",
        ),
        (
            Dimension::ALL.map(|dimension| (dimension, 0.0)).to_vec(),
            "weighs 0",
        ),
    ] {
        let err = Weights::new(set).unwrap_err();
        assert!(err.to_string().contains(message), "{err}");
    }
}
