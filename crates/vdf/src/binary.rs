//! Binary KeyValues, used by PICS package info and `appinfo.vdf`.
//! Numbers are converted to decimal strings so both formats share [`Obj`].

use crate::{Error, Obj, Value};

const OBJECT: u8 = 0x00;
const STRING: u8 = 0x01;
const INT32: u8 = 0x02;
const FLOAT32: u8 = 0x03;
const POINTER: u8 = 0x04;
const COLOR: u8 = 0x06;
const UINT64: u8 = 0x07;
const END: u8 = 0x08;
const INT64: u8 = 0x0A;
const END_ALT: u8 = 0x0B;

pub fn parse(data: &[u8]) -> Result<Obj, Error> {
    let mut r = Reader { data, pos: 0 };
    r.object()
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn object(&mut self) -> Result<Obj, Error> {
        let mut out = Vec::new();
        loop {
            let Some(kind) = self.byte() else { return Ok(Obj(out)) };
            if kind == END || kind == END_ALT {
                return Ok(Obj(out));
            }
            let key = self.cstr()?;
            let value = match kind {
                OBJECT => Value::Obj(self.object()?),
                STRING => Value::Str(self.cstr()?),
                INT32 | POINTER | COLOR => {
                    Value::Str(i32::from_le_bytes(self.take::<4>()?).to_string())
                }
                FLOAT32 => Value::Str(f32::from_le_bytes(self.take::<4>()?).to_string()),
                UINT64 => Value::Str(u64::from_le_bytes(self.take::<8>()?).to_string()),
                INT64 => Value::Str(i64::from_le_bytes(self.take::<8>()?).to_string()),
                other => return Err(Error::Unexpected(other as char, self.pos)),
            };
            out.push((key, value));
        }
    }

    fn byte(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let s = self.data.get(self.pos..self.pos + N).ok_or(Error::Eof)?;
        self.pos += N;
        Ok(s.try_into().unwrap())
    }

    fn cstr(&mut self) -> Result<String, Error> {
        let rest = &self.data[self.pos..];
        let end = rest.iter().position(|&b| b == 0).ok_or(Error::Eof)?;
        self.pos += end + 1;
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_nested() {
        let mut d = vec![0x00];
        d.extend(b"123\0");
        d.push(0x02);
        d.extend(b"packageid\0");
        d.extend(123i32.to_le_bytes());
        d.push(0x00);
        d.extend(b"appids\0");
        d.push(0x02);
        d.extend(b"0\0");
        d.extend(440i32.to_le_bytes());
        d.extend([0x08, 0x08, 0x08]);
        let o = super::parse(&d).unwrap();
        let pkg = o.obj("123").unwrap();
        assert_eq!(pkg.u64("packageid"), Some(123));
        assert_eq!(pkg.obj("appids").unwrap().u64("0"), Some(440));
    }
}
