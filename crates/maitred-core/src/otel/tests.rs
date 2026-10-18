//! ARCHITECTURE.md's History and OpenTelemetry: one `ResourceMetrics` per line in OTLP's JSON shape,
//! checked against the protocol's own types (opentelemetry-proto, test-only).

use opentelemetry_proto::tonic::common::v1::any_value::Value as ProtoValue;
use opentelemetry_proto::tonic::metrics::v1::{
    ResourceMetrics as ProtoResourceMetrics, metric::Data, number_data_point::Value as ProtoNumber,
};
use proptest::prelude::*;

use super::*;

fn finite(value: f64) -> Finite {
    Finite::new(value).unwrap()
}

fn sample() -> ResourceMetrics {
    ResourceMetrics::new(
        vec![
            KeyValue::string("service.name", "maitred"),
            KeyValue::string("host.name", "mbp"),
            KeyValue::string("maitred.boot", "0123456789abcdef"),
        ],
        "0.0.0",
        vec![
            Metric::gauge(
                "system.cpu.utilization",
                "1",
                vec![NumberDataPoint::new(
                    1_760_000_000_123_000_000,
                    Number::Double(finite(0.42)),
                )],
            ),
            Metric::gauge(
                "maitred.queue.waiting",
                "{ticket}",
                vec![
                    NumberDataPoint::new(1_760_000_000_123_000_000, Number::Int(3))
                        .with(KeyValue::string("maitred.class", "heavy")),
                ],
            ),
        ],
    )
}

#[test]
fn a_sample_is_one_line() {
    let line = sample().to_line();

    assert!(!line.contains('\n'), "{line}");
}

#[test]
fn sixty_four_bit_integers_are_written_as_strings() {
    let line = sample().to_line();

    assert!(
        line.contains(r#""timeUnixNano":"1760000000123000000""#),
        "{line}"
    );
    assert!(line.contains(r#""asInt":"3""#), "{line}");
    assert_eq!(
        KeyValue::bool("maitred.thrash", true).to_json(),
        r#"{"key":"maitred.thrash","value":{"boolValue":true}}"#
    );
    assert!(
        KeyValue::int("maitred.processes", i64::MAX)
            .to_json()
            .contains(r#""intValue":"9223372036854775807""#)
    );
}

#[test]
fn fields_are_named_as_otlp_json_names_them() {
    let line = sample().to_line();

    for name in [
        "\"resource\"",
        "\"scopeMetrics\"",
        "\"scope\"",
        "\"metrics\"",
        "\"gauge\"",
        "\"dataPoints\"",
        "\"asDouble\"",
        "\"stringValue\"",
        "\"attributes\"",
    ] {
        assert!(line.contains(name), "{name} in {line}");
    }
}

#[test]
fn the_protocols_own_types_read_it_back_the_same() {
    let ours = sample();
    let theirs: ProtoResourceMetrics =
        serde_json::from_str(&ours.to_line()).expect("opentelemetry-proto reads our line");

    let resource = theirs.resource.expect("a resource");
    assert_eq!(resource.attributes.len(), 3);
    assert_eq!(resource.attributes[0].key, "service.name");
    assert_eq!(
        resource.attributes[0]
            .value
            .as_ref()
            .and_then(|v| v.value.clone()),
        Some(ProtoValue::StringValue("maitred".into()))
    );
    let scope = &theirs.scope_metrics[0];
    assert_eq!(
        scope.scope.as_ref().map(|s| s.name.as_str()),
        Some("maitred")
    );
    let cpu = &scope.metrics[0];
    assert_eq!(cpu.name, "system.cpu.utilization");
    assert_eq!(cpu.unit, "1");
    let Some(Data::Gauge(gauge)) = &cpu.data else {
        panic!("{cpu:?}")
    };
    assert_eq!(
        gauge.data_points[0].time_unix_nano,
        1_760_000_000_123_000_000
    );
    assert_eq!(
        gauge.data_points[0].value,
        Some(ProtoNumber::AsDouble(0.42))
    );
    let Some(Data::Gauge(queue)) = &scope.metrics[1].data else {
        panic!()
    };
    // opentelemetry-proto reads asInt only as a number, though OTLP/JSON writes 64-bit integers
    // as strings (ARCHITECTURE, Packages: why these types are our own). Our string is asserted in
    // sixty_four_bit_integers_are_written_as_strings; theirs must at least see a point here.
    assert_eq!(queue.data_points.len(), 1);
    assert_eq!(queue.data_points[0].attributes[0].key, "maitred.class");
}

#[test]
fn our_types_read_their_own_line_back() {
    let ours = sample();

    assert_eq!(ResourceMetrics::from_line(&ours.to_line()).unwrap(), ours);
}

/// A line with `"timeUnixNano":<time>` and the point's value `<value>`, its one attribute `<attr>`.
fn point_line(time: &str, value: &str, attr: &str) -> String {
    format!(
        r#"{{"resource":{{"attributes":[]}},"scopeMetrics":[{{"scope":{{"name":"maitred","version":"0"}},"metrics":[{{"name":"m","unit":"1","gauge":{{"dataPoints":[{{"attributes":[{{"key":"k","value":{attr}}}],"timeUnixNano":{time},{value}}}]}}}}]}}]}}"#
    )
}

fn the_point(line: &str) -> Result<NumberDataPoint, serde_json::Error> {
    ResourceMetrics::from_line(line)
        .map(|r| r.scope_metrics[0].metrics[0].gauge.data_points[0].clone())
}

#[test]
fn sixty_four_bit_integers_are_read_as_numbers_too() {
    let point = the_point(&point_line("1", r#""asInt":5"#, r#"{"intValue":-7}"#)).unwrap();

    assert_eq!(point.time_unix_nano, 1);
    assert_eq!(point.value, Number::Int(5));
    assert_eq!(point.attributes[0].value, AnyValue::Int(-7));
}

#[test]
fn an_integer_written_as_neither_a_string_nor_a_number_is_refused() {
    for value in [r#""asInt":true"#, r#""asInt":[5]"#, r#""asInt":null"#] {
        assert!(
            the_point(&point_line("1", value, r#"{"stringValue":"v"}"#)).is_err(),
            "{value}"
        );
    }
    assert!(the_point(&point_line("{}", r#""asInt":5"#, r#"{"stringValue":"v"}"#)).is_err());
}

#[test]
fn an_integer_out_of_range_is_refused() {
    assert!(
        the_point(&point_line(
            r#""1""#,
            r#""asInt":"5""#,
            r#"{"stringValue":"v"}"#
        ))
        .is_ok()
    );
    assert!(the_point(&point_line("-1", r#""asInt":5"#, r#"{"stringValue":"v"}"#)).is_err());
    assert!(
        the_point(&point_line(
            r#""1""#,
            r#""asInt":9223372036854775808"#,
            r#"{"stringValue":"v"}"#
        ))
        .is_err()
    );
}

#[test]
fn an_integer_string_with_a_plus_sign_is_refused() {
    // proto3's JSON mapping allows no leading `+`.
    assert!(
        the_point(&point_line(
            r#""1""#,
            r#""asInt":"+5""#,
            r#"{"stringValue":"v"}"#
        ))
        .is_err()
    );
    assert!(
        the_point(&point_line(
            r#""+1""#,
            r#""asInt":"5""#,
            r#"{"stringValue":"v"}"#
        ))
        .is_err()
    );
}

#[test]
fn a_point_with_both_values_or_neither_is_refused() {
    // OTLP's value is a oneof.
    assert!(
        the_point(&point_line(
            r#""1""#,
            r#""asDouble":2.5,"asInt":"5""#,
            r#"{"stringValue":"v"}"#
        ))
        .is_err()
    );
    assert!(
        the_point(&point_line(
            r#""1""#,
            r#""asInt":"5","asDouble":2.5"#,
            r#"{"stringValue":"v"}"#
        ))
        .is_err()
    );
    let neither = point_line(r#""1""#, r#""x":0"#, r#"{"stringValue":"v"}"#);
    assert!(the_point(&neither).is_err());
}

#[test]
fn a_reading_that_isnt_a_number_cant_be_made() {
    // A zero denominator in the sampler is a figure that couldn't be taken (ARCHITECTURE, History
    // and OpenTelemetry), so it never reaches a line.
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(Finite::new(value), None, "{value}");
    }
    assert_eq!(Finite::new(-0.5).map(Finite::get), Some(-0.5));
}

#[test]
fn every_double_written_reads_back_as_written() {
    for value in [0.0, -0.0, f64::MIN_POSITIVE, f64::MAX, f64::MIN, 0.1 + 0.2] {
        let point = NumberDataPoint::new(1, Number::Double(finite(value)));
        let line =
            ResourceMetrics::new(vec![], "0", vec![Metric::gauge("m", "1", vec![point])]).to_line();
        let Number::Double(read) = the_point(&line).unwrap().value else {
            panic!("{line}")
        };
        assert_eq!(read.get().to_bits(), value.to_bits(), "{line}");
    }
}

fn any_value() -> impl Strategy<Value = AnyValue> {
    prop_oneof![
        any::<String>().prop_map(AnyValue::String),
        any::<bool>().prop_map(AnyValue::Bool),
        any::<i64>().prop_map(AnyValue::Int),
        any::<f64>()
            .prop_filter_map("finite", Finite::new)
            .prop_map(AnyValue::Double),
    ]
}

fn number() -> impl Strategy<Value = Number> {
    prop_oneof![
        any::<i64>().prop_map(Number::Int),
        any::<f64>()
            .prop_filter_map("finite", Finite::new)
            .prop_map(Number::Double),
    ]
}

proptest! {
    /// Any point, with any attributes, survives the line both ways and reads the same to the
    /// protocol's types.
    #[test]
    fn any_point_round_trips(
        time in any::<u64>(),
        value in number(),
        attributes in proptest::collection::vec(("[a-z.]{1,12}", any_value()), 0..4),
    ) {
        let mut point = NumberDataPoint::new(time, value);
        for (key, v) in attributes {
            point = point.with(KeyValue { key, value: v });
        }
        let ours = ResourceMetrics::new(vec![], "0", vec![Metric::gauge("m", "1", vec![point])]);
        let line = ours.to_line();
        prop_assert_eq!(ResourceMetrics::from_line(&line).unwrap(), ours.clone());
        let theirs: ProtoResourceMetrics = serde_json::from_str(&line).unwrap();
        let Some(Data::Gauge(g)) = &theirs.scope_metrics[0].metrics[0].data else {
            panic!()
        };
        prop_assert_eq!(g.data_points[0].time_unix_nano, time);
        // As above: opentelemetry-proto reads asInt only as a number, so only a double is
        // compared through it.
        if let Number::Double(d) = value {
            prop_assert_eq!(g.data_points[0].value, Some(ProtoNumber::AsDouble(d.get())));
        }

    }
}
