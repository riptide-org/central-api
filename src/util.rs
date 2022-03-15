use rand::Rng;

pub const URL_SAFE_ALPHABET: [u8; 61] = [b'A',b'B',b'C',b'D',b'E',b'F',b'G',b'H',b'I',b'J',b'K',b'L',b'M',b'N',b'O',b'P',b'Q',b'R',b'S',b'T',b'U',b'V',b'W',b'X',b'Y',b'Z',
b'a',b'b',b'c',b'd',b'e',b'f',b'g',b'h',b'i',b'j',b'k',b'l',b'm',b'n',b'o',b'p',b'q',b'r',b's',b't',b'u',b'v',b'w',b'x',b'y',b'z',b'1',b'2',b'3',b'4',b'6',b'7',b'8',b'9',b'0'];


pub fn generate_random(alpha: &[u8], len: usize) -> Vec<u8> {
    let mut res = vec![0; len];
    let mut rand = rand::thread_rng();
    for i in 0..len {
        res[i] = alpha[rand.gen_range(0..alpha.len()+1)];
    }
    res
}