use cel::extractors::Argument;
use cel::objects::{KeyRef, ListValue, MapValue};
use cel::{Context, FunctionContext, ResolveResult, Value};
use vector_map::VecMap;

pub fn insert_all(ctx: &mut Context) {
	ctx.add_function("flatten", flatten);
	// Keep old and new name for compatibility
	ctx.add_function("flatten_recursive", flatten_recursive);
	ctx.add_function("flattenRecursive", flatten_recursive);
}

pub static FLATTEN_LIST: &str = "$_meta_flatten_list";
pub static FLATTEN_LIST_RECURSIVE: &str = "$_meta_flatten_list_recursive";
pub static FLATTEN_MAP: &str = "$_meta_flatten_map";
pub static FLATTEN_MAP_RECURSIVE: &str = "$_meta_flatten_map_recursive";

#[derive(Clone, Debug)]
pub enum FlattenSignal<'a> {
	Map(MapValue<'a>),
	MapRecursive(MapValue<'a>),
	List(ListValue<'a>),
	ListRecursive(ListValue<'a>),
}
impl<'a> FlattenSignal<'a> {
	pub fn from_value(v: &'a Value<'a>) -> Option<FlattenSignal<'a>> {
		let Value::Map(m) = v else { return None };
		if m.len() != 1 {
			return None;
		}
		let KeyRef::String(k) = m.iter().next().unwrap().0 else {
			return None;
		};
		match k.as_ref() {
			s if s == FLATTEN_LIST => {
				let Value::List(l) = m.iter_owned().next().unwrap().1 else {
					unreachable!()
				};
				Some(FlattenSignal::List(l))
			},
			s if s == FLATTEN_LIST_RECURSIVE => {
				let Value::List(l) = m.iter_owned().next().unwrap().1 else {
					unreachable!()
				};
				Some(FlattenSignal::ListRecursive(l))
			},
			s if s == FLATTEN_MAP => {
				let Value::Map(l) = m.iter_owned().next().unwrap().1 else {
					unreachable!()
				};
				Some(FlattenSignal::Map(l))
			},
			s if s == FLATTEN_MAP_RECURSIVE => {
				let Value::Map(l) = m.iter_owned().next().unwrap().1 else {
					unreachable!()
				};
				Some(FlattenSignal::MapRecursive(l))
			},
			_ => None,
		}
	}
}

fn flatten<'a>(ftx: &mut FunctionContext<'a, '_>, v: Argument) -> ResolveResult<'a> {
	let v = v.load(ftx)?;
	let res = match v {
		Value::List(_) => Value::Map(MapValue::Borrow(VecMap::from_iter([(
			KeyRef::String(FLATTEN_LIST.into()),
			v,
		)]))),
		Value::Map(_) => Value::Map(MapValue::Borrow(VecMap::from_iter([(
			KeyRef::String(FLATTEN_MAP.into()),
			v,
		)]))),
		_ => {
			return ftx.error("flatten only works on Map or List").into();
		},
	};
	res.into()
}

fn flatten_recursive<'a>(ftx: &mut FunctionContext<'a, '_>, v: Argument) -> ResolveResult<'a> {
	let v = v.load(ftx)?;
	let res = match v {
		Value::List(_) => Value::Map(MapValue::Borrow(VecMap::from_iter([(
			KeyRef::String(FLATTEN_LIST_RECURSIVE.into()),
			v,
		)]))),
		Value::Map(_) => Value::Map(MapValue::Borrow(VecMap::from_iter([(
			KeyRef::String(FLATTEN_MAP_RECURSIVE.into()),
			v,
		)]))),
		_ => {
			return ftx.error("flatten only works on Map or List").into();
		},
	};
	res.into()
}
