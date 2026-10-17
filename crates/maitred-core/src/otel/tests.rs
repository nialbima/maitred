//! ARCHITECTURE.md's History and OpenTelemetry: one `ResourceMetrics` per line in OTLP's JSON shape,
//! checked against the protocol's own types (opentelemetry-proto, test-only).

use opentelemetry_proto::tonic::common::v1::any_value::Value as ProtoValue;
use opentelemetry_proto::tonic::metrics::v1::{
    ResourceMetrics as ProtoResourceMetrics, metric::Data, number_data_point::Value as ProtoNumber,
};
use proptest::prelude::*;

use super::*;

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
                    Number::Double(0.42),
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

fn any_value() -> impl Strategy<Value = AnyValue> {
    prop_oneof![
        any::<String>().prop_map(AnyValue::String),
        any::<bool>().prop_map(AnyValue::Bool),
        any::<i64>().prop_map(AnyValue::Int),
        (-1e15..1e15f64).prop_map(AnyValue::Double),
    ]
}

fn number() -> impl Strategy<Value = Number> {
    prop_oneof![
        any::<i64>().prop_map(Number::Int),
        (-1e15..1e15f64).prop_map(Number::Double),
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
            prop_assert_eq!(g.data_points[0].value, Some(ProtoNumber::AsDouble(d)));
        }

    }
}
