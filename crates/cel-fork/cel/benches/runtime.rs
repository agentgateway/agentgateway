use std::collections::HashMap;
use std::hint::black_box;
use std::marker::PhantomData;
use std::time::Duration;

use cel::context::{CompositeResolver, Context, MapResolver, VariableResolver};
use cel::{Program, Value};
use criterion::{BenchmarkId, Criterion, criterion_group};
use pprof::criterion::Output;
use serde::Serialize;

const EXPRESSIONS: [(&str, &str); 35] = [
    ("ternary_1", "(false || true) ? 1 : 2"),
    ("ternary_2", "(true ? false : true) ? 1 : 2"),
    ("or_1", "false || true"),
    ("and_1", "true && false"),
    ("and_2", "true && (false ? 2 : 3) > 2"),
    ("number", "1"),
    ("construct_list", "[1,2,3]"),
    ("construct_list_1", "[1]"),
    ("construct_list_2", "[a, 2]"),
    ("add_list", "[1,2,3] + [4, 5, 6]"),
    ("list_element", "[1,2,3][1]"),
    ("construct_dict", "{1: 2, '3': '4'}"),
    ("add_string", "'abc' + 'def'"),
    ("mapexpr", "{1 + a: 3}"),
    ("size_list", "[1].size()"),
    ("size_list_1", "size([1])"),
    ("size_str", "'a'.size()"),
    ("size_str_2", "size('a')"),
    ("size_map", "{1:2}.size()"),
    ("size_map_2", "size({1:2})"),
    ("member", "foo.bar"),
    ("map has", "has(foo.bar.baz)"),
    ("map macro", "[1, 2, 3].map(x, x * 2)"),
    ("filter macro", "[1, 2, 3].filter(x, x > 2)"),
    ("all macro", "[1, 2, 3].all(x, x > 0)"),
    ("all map macro", "{0: 0, 1:1, 2:2}.all(x, x >= 0)"),
    ("max", "max(1, 2, 3)"),
    ("max negative", "max(-1, 0, 1)"),
    ("max float", "max(-1.0, 0.0, 1.0)"),
    ("duration", "duration('1s')"),
    ("timestamp", "timestamp('2023-05-28T00:00:00Z')"), /* ("complex", "Account{user_id: 123}.user_id == 123"), */
    ("variable resolver", "banana"),
    ("variable hashmap", "apple"),
    (
        "stress",
        "true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true && true",
    ),
    ("regex", "'abc'.matches('^[a-z]*$')"),
];

struct Resolver<'a>(PhantomData<&'a ()>);

impl<'a> VariableResolver<'a> for Resolver<'a> {
    fn resolve(&self, expr: &str) -> Option<Value<'a>> {
        const V: Value = Value::Bool(false);
        const NOT_V: Value = Value::Bool(true);
        match expr {
            "fruit" => Some(NOT_V),
            "carrot" => Some(NOT_V),
            "orange" => Some(NOT_V),
            "banana" => Some(V),
            _ => None,
        }
    }
}

pub fn criterion_benchmark(c: &mut Criterion) {
    // https://gist.github.com/rhnvrm/db4567fcd87b2cb8e997999e1366d406
    let mut execution_group = c.benchmark_group("execute");
    for (name, expr) in black_box(&EXPRESSIONS) {
        execution_group.bench_function(BenchmarkId::from_parameter(name), |b| {
            let program = Program::compile(expr).expect("Parsing failed");
            // eprintln!("{program:#?}");
            let ctx = Context::default();
            let mut vars = MapResolver::new();
            vars.add_variable_from_value("foo", HashMap::from([("bar", 1)]));
            vars.add_variable_from_value("apple", true);
            vars.add_variable_from_value("a", 1);
            let rv = CompositeResolver::new(&vars, &Resolver(PhantomData));
            b.iter(|| Value::resolve(program.expression(), &ctx, &rv).expect("Eval failed!"))
        });
    }
}

pub fn criterion_benchmark_parsing(c: &mut Criterion) {
    let mut parsing_group = c.benchmark_group("parse");
    for (name, expr) in black_box(&EXPRESSIONS) {
        parsing_group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| Program::compile(expr).expect("Parsing failed"))
        });
    }
}

pub fn map_macro_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("map list");
    let sizes = vec![1, 10, 100, 1000, 10000];

    for size in sizes {
        group.bench_function(format!("map_{size}").as_str(), |b| {
            let list = (0..size).collect::<Vec<_>>();
            let program = Program::compile("list.map(x, x * 2)").unwrap();
            let ctx = Context::default();
            let mut vars = MapResolver::new();
            vars.add_variable_from_value("list", list);
            b.iter(|| program.execute_with(&ctx, &vars).expect("Eval failed!"))
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
      .warm_up_time(Duration::from_millis(10))
      .measurement_time(Duration::from_millis(100))
      .with_profiler(pprof::criterion::PProfProfiler::new(100, Output::Protobuf));
    targets = criterion_benchmark, criterion_benchmark_parsing, map_macro_benchmark
}

const REALISTIC_EXPRESSIONS: &[(&str, &str)] = &[];
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RequestOpaque<'a>(&'a HttpRequest);

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// This is the following macro expanded:
/// criterion_main!(benches);
/// But expanded manually so that we can keep the dhat profiler in scope until after benchmarks run
fn main() {
    #[cfg(feature = "dhat-heap")]
    let profiler = dhat::Profiler::new_heap();

    benches();
    // If adding new criterion groups, do so here.

    // Dropping the dhat profiler prints information to stderr: https://docs.rs/dhat/latest/dhat/
    // Doing so before the below ensures profiler doesn't measure Criterion's summary code.
    // It still may measure other bits of Criterion during the benchmark, of course..
    #[cfg(feature = "dhat-heap")]
    drop(profiler);

    Criterion::default().configure_from_args().final_summary();
}
