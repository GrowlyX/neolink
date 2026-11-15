#[link(name = "compression")]
extern "C" { fn compression_decode_buffer(d:*mut u8,n:usize,s:*const u8,sn:usize,sc:*mut std::ffi::c_void,a:i32)->usize; }
fn dec(d:&[u8])->Option<Vec<u8>>{
  let mut o=vec![0u8;8<<20];
  let n=unsafe{compression_decode_buffer(o.as_mut_ptr(),o.len(),d.as_ptr(),d.len(),std::ptr::null_mut(),0x205)};
  if n==0||n>=o.len(){None}else{o.truncate(n);Some(o)}
}
fn main(){
  let p=std::env::args().nth(1).unwrap();
  let k=neolink_core::spkg::parse_file(std::path::Path::new(&p)).unwrap();
  let r=&k.records[0];
  let b=&r.body;
  println!("len {}", b.len());
  for s in [0usize,5] {
    let Some(v)=dec(&b[s..]) else { println!("s={s} FAIL"); continue };
    println!("s={s} out={}", v.len());
    // is v a subsequence of b? find v[..32] in b
    let probe=&v[..32.min(v.len())];
    let pos=(0..b.len().saturating_sub(probe.len())).find(|&i| &b[i..i+probe.len()]==probe);
    println!("  first32={:02x?}", probe);
    println!("  found_at={:?}", pos);
    if let Some(q)=pos {
      println!("  eq_all={}", q+v.len()<=b.len() && &v[..]==&b[q..q+v.len()]);
    }
    println!("  tail={:02x?}", &v[v.len()-8..]);
  }
  println!("body[0..16]={:02x?}", &b[..16]);
}
