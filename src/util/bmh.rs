pub(crate) fn into_needle(pattern: &str) -> Vec<Option<u8>> {
  pattern
    .split(" ")
    .map(|byte| match byte {
      "?" | "??" => None,
      x => u8::from_str_radix(x, 16).ok(),
    })
    .collect::<Vec<_>>()
}

pub(crate) fn bmh(haystack: &[u8], needle: &[Option<u8>]) -> Option<usize> {
  let (m, n) = (needle.len(), haystack.len());
  if m > n {
    return None;
  }

  let mut skip = [m; 256];
  for k in 0..m - 1 {
    if let Some(v) = needle[k] {
      skip[v as usize] = m - k - 1;
    }
  }

  let mut k = m - 1;
  while k < n {
    let mut j = (m - 1) as isize;
    let mut i = k as isize;
    while j >= 0
      && (needle[j as usize].is_none() || needle[j as usize] == Some(haystack[i as usize]))
    {
      j -= 1;
      i -= 1;
    }
    if j < 0 {
      return Some((i + 1) as usize);
    }
    k += skip[haystack[k] as usize];
  }

  None
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_bmh() {
    let haystack = [0u8, 0, 0, 1, 2, 3, 4, 5, 4, 3, 2, 1];
    let needle1 = into_needle("00 00 00 01");
    let needle2 = into_needle("01 00 00 01");
    let needle3 = into_needle("01 02 ?? 04");
    assert_eq!(bmh(&haystack, &needle1), Some(0usize));
    assert_eq!(bmh(&haystack, &needle2), None);
    assert_eq!(bmh(&haystack, &needle3), Some(3usize));
  }

  #[test]
  fn test_into_needle() {
    assert_eq!(
      into_needle("00 11 22 ??"),
      vec![Some(0x00), Some(0x11), Some(0x22), None]
    );
  }
}
