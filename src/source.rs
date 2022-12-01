use std::{collections::HashMap, io::{Lines, BufRead, BufReader}, fs::File};

use crate::isa;

type Opcode = u8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BReg {
    Zero = 0,
    Al = 1,
    Ah = 2,
    Bl = 3,
    Bh = 4,
    Cl = 5,
    Ch = 6,
    Io = 7,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WReg {
    Zero = 0,
    A = 1,
    B = 2,
    C = 3,
    X = 4,
    Y = 5,
    Z = 6,
    S = 7,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceOperand {
    Byte(u8),
    Wide(u16),
    Number(i32),
    ByteReg(BReg),
    WideReg(WReg),
    Label(String),
}

#[derive(Debug, Clone)]
pub enum SourceLine {
    Label(String),
    Ins(String, Vec<SourceOperand>),
    Comment,
    DirInclude(String),
    DirString(Vec<u8>),
    DirByte(u8),
    DirWide(u16),
}

pub struct SourceLines<B> {
    lines: Lines<B>,
}

impl<B: BufRead> SourceLines<B> {
    pub fn new(r: B) -> Self {
        SourceLines {
            lines: r.lines()
        }
    }
}

fn parse_bytechar(s: &[u8]) -> (u8, &[u8]) {
    let mut bs = s.iter();
    match bs.next().unwrap() {
        b'\\' => match bs.next().unwrap() {
            b'r' => (b'\r', &s[2..]),
            b't' => (b'\t', &s[2..]),
            b'n' => (b'\n', &s[2..]),
            b'0' => (b'\0', &s[2..]),
            b'\\' => (b'\\', &s[2..]),
            b'\'' => (b'\'', &s[2..]),
            b'\"' => (b'\"', &s[2..]),
            b'x' => (u8::from_str_radix(String::from_utf8_lossy(&s[2..4]).as_ref(), 16).expect("invalid escape argument"), &s[4..]),
            c => panic!("invalid escape character \\{c}"),
        }
        &c => (c, &s[1..]),
    }
}

impl<B: BufRead> Iterator for SourceLines<B> {
    type Item = SourceLine;
    fn next(&mut self) -> Option<Self::Item> {
        Some(loop {
            let line = self.lines.next()?;
            let line = line.unwrap();
            let line = line.trim();

            if line.is_empty() {
                continue;
            }
            if line.starts_with(";") || line.starts_with("//") {
                break SourceLine::Comment;
            }
            if line.starts_with(".") {
                let line = &line[1..];
                let i = line.find(' ').unwrap_or(line.len());
                let arg = &line[i+1..];
                match &line[..i] {
                    "string" => break SourceLine::DirString({
                        let mut string = Vec::with_capacity(arg.len());
                        let mut arg = arg.as_bytes();
                        while !arg.is_empty() {
                            let (c, rest) = parse_bytechar(arg);
                            arg = rest;
                            string.push(c);
                        }
                        string
                    }),
                    "byte" => break SourceLine::DirByte(arg.parse().unwrap()),
                    "wide" | "word" => break SourceLine::DirWide(arg.parse().unwrap()),
                    "include" => break SourceLine::DirInclude(arg.to_string()),
                    s => panic!("unknown directive {s}"),
                }
            }
            if line.ends_with(":") {
                break SourceLine::Label((line[..line.len()-1]).to_owned())
            }
            if let Some(i) = line.find(' ') {
                let (ins, args) = line.split_at(i);
                let mut sos = Vec::new();

                for arg in args.split(',') {
                    let arg = arg.trim();

                    sos.push(match arg {
                        "al" => SourceOperand::ByteReg(BReg::Al),
                        "ah" => SourceOperand::ByteReg(BReg::Ah),
                        "bl" => SourceOperand::ByteReg(BReg::Bl),
                        "bh" => SourceOperand::ByteReg(BReg::Bh),
                        "cl" => SourceOperand::ByteReg(BReg::Cl),
                        "ch" => SourceOperand::ByteReg(BReg::Ch),
                        "io" => SourceOperand::ByteReg(BReg::Io),
                        "a" => SourceOperand::WideReg(WReg::A),
                        "b" => SourceOperand::WideReg(WReg::B),
                        "c" => SourceOperand::WideReg(WReg::C),
                        "x" => SourceOperand::WideReg(WReg::X),
                        "y" => SourceOperand::WideReg(WReg::Y),
                        "z" => SourceOperand::WideReg(WReg::Z),
                        "s" => SourceOperand::WideReg(WReg::S),
                        arg => {
                            let so;
                            if arg.ends_with("b") {
                                so = arg[..arg.len()-1]
                                    .parse()
                                    .ok()
                                    .or_else(|| arg[..arg.len()-1].parse::<i8>().ok().map(|b| b as u8))
                                    .map(SourceOperand::Byte);
                            } else if arg.ends_with("w") {
                                so = arg[..arg.len()-1]
                                    .parse()
                                    .ok()
                                    .or_else(|| arg[..arg.len()-1].parse::<i16>().ok().map(|w| w as u16))
                                    .map(SourceOperand::Wide);
                            } else if arg.starts_with('\'') && arg.ends_with('\'') {
                                so = Some(SourceOperand::Byte(parse_bytechar(arg[1..arg.len()-1].as_bytes()).0));
                            } else {
                                so = arg.parse().ok().map(SourceOperand::Number);
                            }

                            if let Some(so) = so {
                                so
                            } else {
                                SourceOperand::Label(arg.to_owned())
                            }
                        }
                    });
                }

                break SourceLine::Ins(ins.to_owned(), sos);
            } else {
                break SourceLine::Ins(line.to_owned(), Vec::new());
            }
        })
    }
}

#[derive(Debug, Clone)]
pub enum DataLine {
    Ins(Opcode, DataOperand),
    Raw(Vec<u8>),
}

pub fn process(lines: impl Iterator<Item=SourceLine>) -> (HashMap<usize, u16>, Vec<Box<str>>, Vec<DataLine>) {
    inner_process(lines, &mut 0)
}
fn inner_process(lines: impl Iterator<Item=SourceLine>, cur_offset: &mut u16) -> (HashMap<usize, u16>, Vec<Box<str>>, Vec<DataLine>) {
    let mut data_lines = Vec::new();
    let mut id_to_pos = HashMap::new();
    let mut label_maker = LabelMaker { labels: Vec::new() };

    for line in lines {
        match line {
            SourceLine::Label(s) => {
                let id = label_maker.get_id(&s);
                id_to_pos.insert(id, *cur_offset);
            }
            SourceLine::Ins(s, ops) => {
                let (opcode, dat_op) = parse_ins(s, ops, &mut label_maker);
                *cur_offset += 1 + dat_op.size();
                data_lines.push(DataLine::Ins(opcode, dat_op));
            }
            SourceLine::DirByte(b) => {
                *cur_offset += 1;
                data_lines.push(DataLine::Raw(vec![b]));
            }
            SourceLine::DirWide(w) => {
                let [l, h] = w.to_le_bytes();
                *cur_offset += 2;
                data_lines.push(DataLine::Raw(vec![l, h]));
            }
            SourceLine::DirString(s) => {
                *cur_offset += s.len() as u16;
                data_lines.push(DataLine::Raw(s));
            }
            SourceLine::DirInclude(path) => {
                let f = File::open(&path).unwrap();
                let lines = SourceLines::new(BufReader::new(f));
                let (included_id_to_pos, included_labels, included_data_lines) = inner_process(lines, cur_offset);

                data_lines.extend(included_data_lines);
                for (i, lbl) in included_labels.into_iter().enumerate() {
                    let lbl = if lbl.chars().next().unwrap().is_uppercase() {
                        lbl
                    } else {
                        format!("{path}  {lbl}").into_boxed_str()
                    };
                    let new_id = label_maker.get_id(&lbl);
                    id_to_pos.insert(new_id, included_id_to_pos[&i]);
                }
            }
            SourceLine::Comment => (),
        }
    }

    (id_to_pos, label_maker.labels, data_lines)
}

fn parse_ins(s: String, ops: Vec<SourceOperand>, lbl_mkr: &mut LabelMaker) -> (u8, DataOperand) {
    use self::isa::*;
    use self::DataOperand as O;
    let ops = ops.iter();
    match &*s {
        "null" => (NULL, O::parse_nothing(ops).expect("nothing")),
        "halt" => (HALT, O::parse_nothing(ops).expect("nothing")),
        "nop" => (NOP, O::parse_nothing(ops).expect("nothing")),
        "push" => {
            if let Some(dat_op) = O::parse_b_big_r(ops.clone()) {
                (PUSH_B, dat_op)
            } else if let Some(dat_op) = O::parse_w_big_r(ops, lbl_mkr) {
                (PUSH_W, dat_op)
            } else {
                panic!("takes one big");
            }
        }
        "pop" => {
            if let Some(dat_op) = O::parse_breg(ops.clone()) {
                (POP_B, dat_op)
            } else if let Some(dat_op) = O::parse_wreg(ops) {
                (POP_W, dat_op)
            } else {
                panic!("takes one big");
            }
        }
        "call" => (CALL, O::parse_immediate_u16(ops, lbl_mkr).expect("a wide (addr like a label or just a number)")),
        "ret" => (RET, O::parse_nothing(ops.clone()).map(|_| DataOperand::ImmediateByte(0)).or_else(|| O::parse_immediate_u8(ops)).expect("either nothing or a byte")),
        "store" => {
            if let Some(dat_op) = O::parse_wide_big_byte(ops.clone(), lbl_mkr) {
                (STORE_B, dat_op)
            } else if let Some(dat_op) = O::parse_wide_big_wide(ops, lbl_mkr) {
                (STORE_W, dat_op)
            } else {
                panic!("a wide and a big for destination and a source register (any size)");
            }
        }
        "load" => {
             if let Some(dat_op) = O::parse_byte_wide_big(ops.clone(), lbl_mkr) {
                (LOAD_B, dat_op)
            } else if let Some(dat_op) = O::parse_two_wide_one_big(ops, lbl_mkr) {
                (LOAD_W, dat_op)
            } else {
                panic!("a destination register (any size) and then a wide and a big");
            }
        }
        "jmp" | "jump" => {
             if let Some(dat_op) = O::parse_immediate_u16(ops.clone(), lbl_mkr) {
                (JUMP, dat_op)
            } else if let Some(dat_op) = O::parse_wreg(ops) {
                (JUMP_REG, dat_op)
            } else {
                panic!("address or wide register");
            }
        }

        "jez" => (JEZ, O::parse_immediate_u16(ops, lbl_mkr).expect("a wide (addr like a label or just a number)")),
        "jlt" => (JLT, O::parse_immediate_u16(ops, lbl_mkr).expect("a wide (addr like a label or just a number)")),
        "jle" => (JLE, O::parse_immediate_u16(ops, lbl_mkr).expect("a wide (addr like a label or just a number)")),
        "jgt" => (JGT, O::parse_immediate_u16(ops, lbl_mkr).expect("a wide (addr like a label or just a number)")),
        "jge" => (JGE, O::parse_immediate_u16(ops, lbl_mkr).expect("a wide (addr like a label or just a number)")),
        "jnz" => (JNZ, O::parse_immediate_u16(ops, lbl_mkr).expect("a wide (addr like a label or just a number)")),
        "jo" => (JO, O::parse_immediate_u16(ops, lbl_mkr).expect("a wide (addr like a label or just a number)")),
        "jno" => (JNO, O::parse_immediate_u16(ops, lbl_mkr).expect("a wide (addr like a label or just a number)")),
        "jb" | "jc" => (JB, O::parse_immediate_u16(ops, lbl_mkr).expect("a wide (addr like a label or just a number)")),
        "jae" | "jnc" => (JAE, O::parse_immediate_u16(ops, lbl_mkr).expect("a wide (addr like a label or just a number)")),
        "ja" => (JA, O::parse_immediate_u16(ops, lbl_mkr).expect("a wide (addr like a label or just a number)")),
        "jbe" => (JBE, O::parse_immediate_u16(ops, lbl_mkr).expect("a wide (addr like a label or just a number)")),

        "add" => {
            if let Some(dat_op) = O::parse_two_byte_one_big(ops.clone()) {
                (ADD_B, dat_op)
            } else if let Some(dat_op) = O::parse_two_wide_one_big(ops, lbl_mkr) {
                (ADD_W, dat_op)
            } else {
                panic!("two regs and one big");
            }
        }
        "sub" => {
            if let Some(dat_op) = O::parse_two_byte_one_big(ops.clone()) {
                (SUB_B, dat_op)
            } else if let Some(dat_op) = O::parse_two_wide_one_big(ops, lbl_mkr) {
                (SUB_W, dat_op)
            } else {
                panic!("two regs and one big");
            }
        }
        "and" => {
            if let Some(dat_op) = O::parse_two_byte_one_big(ops.clone()) {
                (AND_B, dat_op)
            } else if let Some(dat_op) = O::parse_two_wide_one_big(ops, lbl_mkr) {
                (AND_W, dat_op)
            } else {
                panic!("two regs and one big");
            }
        }
        "or" => {
            if let Some(dat_op) = O::parse_two_byte_one_big(ops.clone()) {
                (OR_B, dat_op)
            } else if let Some(dat_op) = O::parse_two_wide_one_big(ops, lbl_mkr) {
                (OR_W, dat_op)
            } else {
                panic!("two regs and one big");
            }
        }
        "xor" => {
            if let Some(dat_op) = O::parse_two_byte_one_big(ops.clone()) {
                (XOR_B, dat_op)
            } else if let Some(dat_op) = O::parse_two_wide_one_big(ops, lbl_mkr) {
                (XOR_W, dat_op)
            } else {
                panic!("two regs and one big");
            }
        }
        "mul" => {
            if let Some(dat_op) = O::parse_four_byte(ops.clone()) {
                (MUL_B, dat_op)
            } else if let Some(dat_op) = O::parse_four_wide(ops) {
                (MUL_W, dat_op)
            } else {
                panic!("four registers")
            }
        }
        "div" => {
            if let Some(dat_op) = O::parse_four_byte(ops.clone()) {
                (DIV_B, dat_op)
            } else if let Some(dat_op) = O::parse_four_wide(ops) {
                (DIV_W, dat_op)
            } else {
                panic!("four registers")
            }
        }
        _ => panic!("unknown instruction {s}"),
    }
}

fn big_r_to_byte(br: BBigR) -> u8 {
    match br {
        BBigR::Register(r) => r as u8,
        BBigR::Byte(0) => BReg::Zero as u8,
        // Since this b is a number from 1 up to 247, we can just add 7 to encode it between 0x08 and 0xff
        BBigR::Byte(b) => b.checked_add(7).expect("immediate between 1-247"),
    }
}
fn big_r_to_wide(wr: WBigR, id_to_pos: &HashMap<usize, u16>) -> [u8; 2] {
    match wr {
        WBigR::Register(r) => r as u16,
        WBigR::Wide(w) => {
            let w = parse_wide(w, id_to_pos);
            if w == 0 {
                WReg::Zero as u16
            } else {
                // Since this w is a number from 1 up to 65527, we can just add 7 to encode it between 0x08 and 0xffff
                w.checked_add(7).expect("immediate between 1-247")
            }
        }
    }.to_le_bytes()
}

fn parse_wide(w: Wide, id_to_pos: &HashMap<usize, u16>) -> u16 {
    match w {
        Wide::Label(l) => *id_to_pos.get(&l).expect("no such label"),
        Wide::Number(n) => n,
    }
}

pub fn write_data_operand(mem: &mut Vec<u8>, id_to_pos: &HashMap<usize, u16>, dat_op: DataOperand) {
    use self::DataOperand::*;

    match dat_op {
        Nothing => (),
        ByteBigR(br) => mem.push(big_r_to_byte(br)),
        WideBigR(wr) => mem.extend_from_slice(&big_r_to_wide(wr, id_to_pos)),
        ByteRegister(r) => mem.push((r as u8) << 4),
        WideRegister(r) => mem.push((r as u8) << 4),
        ImmediateByte(b) => {
            mem.push(b);
        }
        ImmediateWide(w) => {
            mem.extend_from_slice(&parse_wide(w, id_to_pos).to_le_bytes());
        }
        TwoByteOneBig(r1, r2, br) => {
            mem.push(((r1 as u8) << 4) | r2 as u8);
            mem.push(big_r_to_byte(br));
        }
        WideBigByte(r1, wr, r2) => {
            mem.push(((r1 as u8) << 4) | r2 as u8);
            mem.extend_from_slice(&big_r_to_wide(wr, id_to_pos));
        }
        ByteWideBig(r1, r2, wr) => {
            mem.push(((r1 as u8) << 4) | r2 as u8);
            mem.extend_from_slice(&big_r_to_wide(wr, id_to_pos));
        }
        WideBigWide(r1, wr, r2) => {
            mem.push(((r1 as u8) << 4) | r2 as u8);
            mem.extend_from_slice(&big_r_to_wide(wr, id_to_pos));
        }
        TwoWideOneBig(r1, r2, wr) => {
            mem.push(((r1 as u8) << 4) | r2 as u8);
            mem.extend_from_slice(&big_r_to_wide(wr, id_to_pos));
        }
        FourByte(r1, r2, r3, r4) => {
            mem.push(((r1 as u8) << 4) | r2 as u8);
            mem.push(((r3 as u8) << 4) | r4 as u8);
        }
        FourWide(r1, r2, r3, r4) => {
            mem.push(((r1 as u8) << 4) | r2 as u8);
            mem.push(((r3 as u8) << 4) | r4 as u8);
        }
    }
}

struct LabelMaker {
    labels: Vec<Box<str>>,
}

impl LabelMaker {
    fn get_id(&mut self, lbl: &str) -> usize {
        if let Some(i) = self.labels.iter().position(|l| &**l == lbl) {
            i
        } else {
            let i = self.labels.len();
            self.labels.push(lbl.to_owned().into_boxed_str());
            i
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Wide {
    Number(u16),
    Label(usize),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BBigR {
    Register(BReg),
    Byte(u8),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum WBigR {
    Register(WReg),
    Wide(Wide),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DataOperand {
    Nothing,
    ByteBigR(BBigR),
    WideBigR(WBigR),
    ByteRegister(BReg),
    WideRegister(WReg),
    ImmediateByte(u8),
    ImmediateWide(Wide),
    TwoByteOneBig(BReg, BReg, BBigR),
    TwoWideOneBig(WReg, WReg, WBigR),
    WideBigWide(WReg, WBigR, WReg),
    ByteWideBig(BReg, WReg, WBigR),
    WideBigByte(WReg, WBigR, BReg),
    FourByte(BReg, BReg, BReg, BReg),
    FourWide(WReg, WReg, WReg, WReg),
}

impl DataOperand {
    fn size(&self) -> u16 {
        use self::DataOperand::*;
        match self {
            Nothing => 0,
            ByteBigR(_) => 1,
            WideBigR(_) => 2,
            ByteRegister(_) => 1,
            WideRegister(_) => 1,
            ImmediateByte(_) => 1,
            ImmediateWide(_) => 2,
            TwoByteOneBig(_, _, _) => 2,
            TwoWideOneBig(_, _, _) => 3,
            WideBigWide(_, _, _) => 3,
            ByteWideBig(_, _, _) => 3,
            WideBigByte(_, _, _) => 3,
            FourByte(_, _, _, _) => 2,
            FourWide(_, _, _, _) => 2,
        }
    }
    fn parse_nothing<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>) -> Option<DataOperand> {
        if ops.next().is_none() {
            Some(DataOperand::Nothing)
        } else { None }
    }
    fn parse_breg<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>) -> Option<DataOperand> {
        let breg = Self::byte(ops.next()?)?;
        Self::parse_nothing(ops)?;
        Some(DataOperand::ByteRegister(breg))
    }
    fn parse_wreg<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>) -> Option<DataOperand> {
        let wreg = Self::wide(ops.next()?)?;
        Self::parse_nothing(ops)?;
        Some(DataOperand::WideRegister(wreg))
    }
    fn parse_immediate_u8<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>) -> Option<DataOperand> {
        let ret = Some(DataOperand::ImmediateByte(Self::imm_byte(ops.next()?)?));
        Self::parse_nothing(ops)?;
        ret
    }
    fn parse_immediate_u16<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>, lbl_mkr: &mut LabelMaker) -> Option<DataOperand> {
        let ret = Some(DataOperand::ImmediateWide(Self::imm_wide(ops.next()?, lbl_mkr)?));
        Self::parse_nothing(ops)?;
        ret
    }
    fn parse_b_big_r<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>) -> Option<DataOperand> {
        let ret = Some(DataOperand::ByteBigR(Self::byte_or_imm(ops.next()?)?));
        Self::parse_nothing(ops)?;
        ret
    }
    fn parse_w_big_r<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>, lbl_mkr: &mut LabelMaker) -> Option<DataOperand> {
        let ret = Some(DataOperand::WideBigR(Self::wide_or_imm(ops.next()?, lbl_mkr)?));
        Self::parse_nothing(ops)?;
        ret
    }
    fn parse_two_byte_one_big<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>) -> Option<DataOperand> {
        let reg1 = ops.next()?;
        let reg2 = ops.next()?;
        Some(DataOperand::TwoByteOneBig(Self::byte(reg1)?, Self::byte(reg2)?, Self::byte_or_imm(ops.next()?)?))
    }
    fn parse_two_wide_one_big<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>, lbl_mkr: &mut LabelMaker) -> Option<DataOperand> {
        let reg1 = ops.next()?;
        let reg2 = ops.next()?;
        Some(DataOperand::TwoWideOneBig(Self::wide(reg1)?, Self::wide(reg2)?, Self::wide_or_imm(ops.next()?, lbl_mkr)?))
    }
    fn parse_wide_big_byte<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>, lbl_mkr: &mut LabelMaker) -> Option<DataOperand> {
        Some(DataOperand::WideBigByte(
            Self::wide(ops.next()?)?,
            Self::wide_or_imm(ops.next()?, lbl_mkr)?,
            Self::byte(ops.next()?)?,
        ))
    }
    fn parse_wide_big_wide<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>, lbl_mkr: &mut LabelMaker) -> Option<DataOperand> {
        Some(DataOperand::WideBigWide(
            Self::wide(ops.next()?)?,
            Self::wide_or_imm(ops.next()?, lbl_mkr)?,
            Self::wide(ops.next()?)?,
        ))
    }
    fn parse_byte_wide_big<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>, lbl_mkr: &mut LabelMaker) -> Option<DataOperand> {
        Some(DataOperand::ByteWideBig(
            Self::byte(ops.next()?)?,
            Self::wide(ops.next()?)?,
            Self::wide_or_imm(ops.next()?, lbl_mkr)?,
        ))
    }
    fn parse_four_byte<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>) -> Option<DataOperand> {
        let reg1 = ops.next()?;
        let reg2 = ops.next()?;
        let reg3 = ops.next()?;
        let reg4 = ops.next()?;
        Self::parse_nothing(ops);
        Some(DataOperand::FourByte(Self::byte(reg1)?, Self::byte(reg2)?, Self::byte(reg3)?, Self::byte(reg4)?))
    }
    fn parse_four_wide<'a>(mut ops: impl Iterator<Item=&'a SourceOperand>) -> Option<DataOperand> {
        let reg1 = ops.next()?;
        let reg2 = ops.next()?;
        let reg3 = ops.next()?;
        let reg4 = ops.next()?;
        Self::parse_nothing(ops);
        Some(DataOperand::FourWide(Self::wide(reg1)?, Self::wide(reg2)?, Self::wide(reg3)?, Self::wide(reg4)?))
    }

    fn byte(op: &SourceOperand) -> Option<BReg> {
        match op {
            SourceOperand::Number(0) => Some(BReg::Zero),
            &SourceOperand::ByteReg(r) => Some(r),
            _ => None,
        }
    }
    fn wide(op: &SourceOperand) -> Option<WReg> {
        match op {
            SourceOperand::Number(0) => Some(WReg::Zero),
            &SourceOperand::WideReg(r) => Some(r),
            _ => None,
        }
    }
    fn imm_byte(op: &SourceOperand) -> Option<u8> {
        match op {
            &SourceOperand::Number(n) => Some(n as u8),
            &SourceOperand::Byte(n) => Some(n),
            _ => None,
        }
    }
    fn imm_wide(op: &SourceOperand, lbl_mkr: &mut LabelMaker) -> Option<Wide> {
        match op {
            &SourceOperand::Number(n) => Some(Wide::Number(n as u16)),
            &SourceOperand::Wide(n) => Some(Wide::Number(n)),
            SourceOperand::Label(lbl) => Some(Wide::Label(lbl_mkr.get_id(lbl))),
            _ => None,
        }
    }
    fn byte_or_imm(op: &SourceOperand) -> Option<BBigR> {
        Self::byte(op)
            .map(BBigR::Register)
            .or_else(|| Self::imm_byte(op).map(BBigR::Byte))
    }
    fn wide_or_imm(op: &SourceOperand, lbl_mkr: &mut LabelMaker) -> Option<WBigR> {
        Self::wide(op)
            .map(WBigR::Register)
            .or_else(|| Self::imm_wide(op, lbl_mkr).map(WBigR::Wide))
    }
}
