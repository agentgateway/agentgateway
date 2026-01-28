use chrono::Duration;
use nom::IResult;
use nom::branch::alt;
use nom::bytes::complete::tag;
use nom::character::complete::char;
use nom::combinator::{map, opt};
use nom::multi::many1;
use nom::number::complete::double;

// Constants representing time units in nanoseconds
const SECOND: u64 = 1_000_000_000;
const MILLISECOND: u64 = 1_000_000;
const MICROSECOND: u64 = 1_000;

/// Parses a duration string into a [`Duration`]. Duration strings support the
/// following grammar:
///
/// DurationString -> Sign? Number Unit String?
/// Sign           -> '-'
/// Number         -> Digit+ ('.' Digit+)?
/// Digit          -> '0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9'
/// Unit           -> 'h' | 'm' | 's' | 'ms' | 'us' | 'ns'
/// String         -> DurationString
///
/// # Examples
/// - `1h` parses as 1 hour
/// - `1.5h` parses as 1 hour and 30 minutes
/// - `1h30m` parses as 1 hour and 30 minutes
/// - `1h30m1s` parses as 1 hour, 30 minutes, and 1 second
/// - `1ms` parses as 1 millisecond
/// - `1.5ms` parses as 1 millisecond and 500 microseconds
/// - `1ns` parses as 1 nanosecond
/// - `1.5ns` parses as 1 nanosecond (sub-nanosecond durations not supported)
pub fn parse_duration(i: &str) -> IResult<&str, Duration> {
    let (i, neg) = opt(parse_negative)(i)?;
    if i == "0" {
        return Ok((i, Duration::zero()));
    }
    let (i, duration) = many1(parse_number_unit)(i)
        .map(|(i, d)| (i, d.iter().fold(Duration::zero(), |acc, next| acc + *next)))?;
    Ok((i, duration * if neg.is_some() { -1 } else { 1 }))
}

enum Unit {
    Nanosecond,
    Microsecond,
    Millisecond,
    Second,
    Minute,
    Hour,
}

impl Unit {
    fn nanos(&self) -> i64 {
        match self {
            Unit::Nanosecond => 1,
            Unit::Microsecond => 1_000,
            Unit::Millisecond => 1_000_000,
            Unit::Second => 1_000_000_000,
            Unit::Minute => 60 * 1_000_000_000,
            Unit::Hour => 60 * 60 * 1_000_000_000,
        }
    }
}

fn parse_number_unit(i: &str) -> IResult<&str, Duration> {
    let (i, num) = double(i)?;
    let (i, unit) = parse_unit(i)?;
    let duration = to_duration(num, unit);
    Ok((i, duration))
}

fn parse_negative(i: &str) -> IResult<&str, ()> {
    let (i, _): (&str, char) = char('-')(i)?;
    Ok((i, ()))
}

fn parse_unit(i: &str) -> IResult<&str, Unit> {
    alt((
        map(tag("ms"), |_| Unit::Millisecond),
        map(tag("us"), |_| Unit::Microsecond),
        map(tag("ns"), |_| Unit::Nanosecond),
        map(char('h'), |_| Unit::Hour),
        map(char('m'), |_| Unit::Minute),
        map(char('s'), |_| Unit::Second),
    ))(i)
}

fn to_duration(num: f64, unit: Unit) -> Duration {
    Duration::nanoseconds((num * unit.nanos() as f64).trunc() as i64)
}

/// Formats a [`Duration`] into a string. String returns a string representing the
/// duration in the form "72h3m0.5s". Leading zero units are omitted. As a special
/// case, durations less than one second format use a smaller unit (milli-, micro-,
/// or nanoseconds) to ensure that the leading digit is non-zero. The zero duration
/// formats as 0s.
///
/// This is a direct port of the Go version of the time.Duration(0).String() function.
pub fn format_duration(d: &Duration) -> String {
    let buf = &mut [0u8; 32];
    let mut w = buf.len();

    let mut neg = false;
    let mut u = d
        .num_nanoseconds()
        .map(|n| {
            if n < 0 {
                neg = true;
            }
            n as u64
        })
        .unwrap_or_else(|| {
            let s = d.num_seconds();
            if s < 0 {
                neg = true;
            }
            s as u64 * SECOND
        });

    if u < SECOND {
        // Special case: if duration is smaller than a second,
        // use smaller units, like 1.2ms
        let mut _prec = 0;
        w -= 1;
        buf[w] = b's';
        w -= 1;

        if u == 0 {
            return "0s".to_string();
        } else if u < MICROSECOND {
            _prec = 0;
            buf[w] = b'n';
        } else if u < MILLISECOND {
            _prec = 3;
            // U+00B5 'µ' micro sign == 0xC2 0xB5
            buf[w] = 0xB5;
            w -= 1;
            buf[w] = 0xC2;
        } else {
            _prec = 6;
            buf[w] = b'm';
        }
        (w, u) = format_float(&mut buf[..w], u, _prec);
        w = format_int(&mut buf[..w], u);
    } else {
        w -= 1;
        buf[w] = b's';
        (w, u) = format_float(&mut buf[..w], u, 9);

        // u is now integer number of seconds
        w = format_int(&mut buf[..w], u % 60);
        u /= 60;

        // u is now integer number of minutes
        if u > 0 {
            w -= 1;
            buf[w] = b'm';
            w = format_int(&mut buf[..w], u % 60);
            u /= 60;

            // u is now integer number of hours
            if u > 0 {
                w -= 1;
                buf[w] = b'h';
                w = format_int(&mut buf[..w], u);
            }
        }
    }

    if neg {
        w -= 1;
        buf[w] = b'-';
    }
    String::from_utf8_lossy(&buf[w..]).into_owned()
}

fn format_float(buf: &mut [u8], mut v: u64, prec: usize) -> (usize, u64) {
    let mut w = buf.len();
    let mut print = false;
    for _ in 0..prec {
        let digit = v % 10;
        print = print || digit != 0;
        if print {
            w -= 1;
            buf[w] = digit as u8 + b'0';
        }
        v /= 10;
    }
    if print {
        w -= 1;
        buf[w] = b'.';
    }
    (w, v)
}

fn format_int(buf: &mut [u8], mut v: u64) -> usize {
    let mut w = buf.len();
    if v == 0 {
        w -= 1;
        buf[w] = b'0';
    } else {
        while v > 0 {
            w -= 1;
            buf[w] = (v % 10) as u8 + b'0';
            v /= 10;
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use crate::duration::{format_duration, parse_duration};

    fn assert_duration(input: &str, expected: Duration) {
        let (_, duration) = parse_duration(input).unwrap();
        assert_eq!(duration, expected, "{input}");
    }

    fn assert_print_duration(input: Duration, expected: &str) {
        let actual = format_duration(&input);
        assert_eq!(actual, expected, "{input}");
    }

    macro_rules! assert_durations {
        ($($str:expr => $duration:expr),*$(,)?) => {
            #[test]
            fn test_durations() {
                $(
                    assert_duration($str, $duration);
                )*
            }
        };
    }

    macro_rules! assert_duration_format {
        ($($duration:expr => $str:expr),*$(,)?) => {
            #[test]
            fn test_format_durations() {
                $(
                    assert_print_duration($duration, $str);
                )*
            }
        };
    }

    assert_durations! {
        "1s" => Duration::seconds(1),
        "-1s" => Duration::seconds(-1),
        "1.1s" => Duration::seconds(1) + Duration::milliseconds(100),
        "1.5m" => Duration::minutes(1) + Duration::seconds(30),
        "1m1s" => Duration::minutes(1) + Duration::seconds(1),
        "1h1m1s" => Duration::hours(1) + Duration::minutes(1) + Duration::seconds(1),
        "1ms" => Duration::milliseconds(1),
        "1us" => Duration::microseconds(1),
        "1ns" => Duration::nanoseconds(1),
        "1.1ns" => Duration::nanoseconds(1),
        "1.123us" => Duration::microseconds(1) + Duration::nanoseconds(123),
        "0s" => Duration::zero(),
        "0h0m0s" => Duration::zero(),
        "0h0m1s" => Duration::seconds(1),
        "0" => Duration::zero(),
        "-0" => Duration::zero(),
    }

    assert_duration_format! {
        Duration::zero() => "0s",
        Duration::nanoseconds(1) => "1ns",
        Duration::nanoseconds(1100) => "1.1µs",
        Duration::microseconds(2200) => "2.2ms",
        Duration::milliseconds(3300) => "3.3s",
        Duration::minutes(4) + Duration::seconds(5) => "4m5s",
        Duration::minutes(4) + Duration::milliseconds(5001) => "4m5.001s",
        Duration::hours(5) + Duration::minutes(6) + Duration::milliseconds(7001) => "5h6m7.001s",
        Duration::minutes(8) + Duration::nanoseconds(1) => "8m0.000000001s",
        Duration::nanoseconds(i64::MAX) => "2562047h47m16.854775807s",
        Duration::nanoseconds(i64::MIN) => "-2562047h47m16.854775808s",
    }
}
