use std::collections::HashMap;
use std::collections::HashSet;

use std::io::BufReader;
use std::io::Read;
use std::io::Seek;

use super::file::*;
use super::FormatToken;
use crate::strings::*;

#[derive(Debug)]
pub (crate) struct Link {
    base : u32,
    check : u32
}

impl Link {
    pub (crate) fn read<T : Read>(sysdic : &mut BufReader<T>) -> Result<Link, &'static str>
    {
        Ok(Link{base : read_u32(sysdic)?, check : read_u32(sysdic)?})
    }
}

fn check_valid_link(links : &[Link], from : u32, to : u32) -> Result<(), i32>
{
    // check for overflow
    if to as usize >= links.len()
    {
        return Err(1);
    }
    // make sure we didn't follow a link from somewhere we weren't supposed to
    else if links[to as usize].check != from
    {
        return Err(2);
    }
    // make sure we don't follow a link back where we started
    else if links[to as usize].base == from
    {
        return Err(3);
    }
    Ok(())
}

fn check_valid_out(links : &[Link], from : u32, to : u32) -> Result<(), i32>
{
    if let Err(err) = check_valid_link(links, from, to)
    {
        return Err(err);
    }
    // don't follow links to bases that aren't outputs
    else if links[to as usize].base < 0x8000_0000
    {
        return Err(-1);
    }
    Ok(())
}

fn collect_links_hashmap(links : &[Link], base : u32, collection : &mut Vec<(String, u32)>, key : &[u8])
{
    if check_valid_out(links, base, base).is_ok()
    {
        if let Ok(key) = read_str_buffer(&key)
        {
            collection.push((key, !links[base as usize].base));
        }
    }
    for i in 0..0x100
    {
        if check_valid_link(links, base, base+1+i).is_ok()
        {
            let mut newkey = key.to_owned();
            newkey.push(i as u8);
            collect_links_hashmap(links, links[(base+1+i) as usize].base, collection, &newkey);
        }
    }
}

fn entries_to_tokens(entries : Vec<(String, u32)>, tokens : &[FormatToken]) -> HashMap<String, Vec<FormatToken>>
{
    let mut dictionary : HashMap<String, Vec<FormatToken>> = HashMap::new();
    for entry in entries
    {
        let mut similar_lexemes : Vec<FormatToken> = Vec::new();
        
        let first : u32 = entry.1 / 0x100;
        let count : u32 = entry.1 % 0x100;
        for i in 0..count
        {
            similar_lexemes.push(tokens[(first+i) as usize].clone());
        }
        dictionary.insert(entry.0, similar_lexemes);
    }
    
    dictionary
}

fn collect_links_into_hashmap(links : Vec<Link>, tokens : Vec<FormatToken>) -> HashMap<String, Vec<FormatToken>>
{
    let mut collection : Vec<(String, u32)> = Vec::new();
    collect_links_hashmap(&links, links[0].base, &mut collection, &[]);
    
    entries_to_tokens(collection, &tokens)
}

#[derive(Debug)]
pub (crate) struct DartDict {
    pub(crate) dict: HashMap<String, Vec<FormatToken>>,
    pub(crate) contains_longer: HashSet<String>,
    pub(crate) left_contexts: u32,
    pub(crate) right_contexts: u32,
    pub(crate) feature_bytes: Vec<u8>,
}

impl DartDict {
    pub (crate) fn may_contain(&self, find : &String) -> bool
    {
        self.contains_longer.contains(find) || self.dict.contains_key(find)
    }
    pub (crate) fn dic_get<'a>(&'a self, find : &String) -> Option<&'a Vec<FormatToken>>
    {
        self.dict.get(find)
    }
    pub (crate) fn feature_get(&self, offset : u32) -> Result<String, &'static str>
    {
        if (offset as usize) < self.feature_bytes.len()
        {
            read_str_buffer(&self.feature_bytes[offset as usize..])
        }
        else
        {
            Ok("".to_string())
        }
    }
}

pub (crate) fn load_mecab_dart_file<T : Read + Seek>(arg_magic : u32, dic : &mut BufReader<T>) -> Result<DartDict, &'static str>
{
    // magic
    let magic = read_u32(dic)?;
    if magic != arg_magic
    {
        return Err("not a mecab dic file or is a dic file of the wrong kind");
    }
    
    // 0x04
    let version = read_u32(dic)?;
    if version != 0x66
    {
        return Err("unsupported version");
    }

    // 0x08
    seek_rel_4(dic)?; // dict type - u32 sys (0), usr (1), unk (2) - we don't care and have no use for the information
    
    let _num_unknown = read_u32(dic)?; // number of unique somethings; might be unique lexeme surfaces, might be feature strings, we don't need it
    // 0x10
    // this information is duplicated in the matrix file and we will ensure that it is consistent
    let left_contexts  = read_u32(dic)?;
    let right_contexts = read_u32(dic)?;
    
    // 0x18
    let linkbytes = read_u32(dic)?; // number of bytes used to store the dual-array trie
    if linkbytes%8 != 0
    {
        return Err("dictionary broken: link table stored with number of bytes that is not a multiple of 8");
    }
    let tokenbytes = read_u32(dic)?; // number of bytes used to store the list of tokens
    if tokenbytes%16 != 0
    {
        return Err("dictionary broken: token table stored with number of bytes that is not a multiple of 16");
    }
    // 0x20
    let featurebytes = read_u32(dic)?; // number of bytes used to store the feature string pile
    seek_rel_4(dic)?;
    
    let encoding = read_nstr(dic, 0x20)?;
    if encoding != "UTF-8"
    {
        return Err("only UTF-8 dictionaries are supported. stop using legacy encodings for infrastructure!");
    }
    
    let mut links : Vec<Link> = Vec::with_capacity((linkbytes/8) as usize);
    for _i in 0..(linkbytes/8)
    {
        links.push(Link::read(dic)?);
    }
    
    let mut tokens : Vec<FormatToken> = Vec::with_capacity((tokenbytes/16) as usize);
    for _i in 0..(tokenbytes/16)
    {
        tokens.push(FormatToken::read(dic, tokens.len() as u32)?);
    }
    
    let mut feature_bytes : Vec<u8> = Vec::with_capacity(featurebytes as usize);
    feature_bytes.resize(featurebytes as usize, 0);
    
    if dic.read_exact(&mut feature_bytes).is_err()
    {
        return Err("IO error")
    }
    
    let dictionary = collect_links_into_hashmap(links, tokens);
    
    let mut contains_longer : HashSet<String> = HashSet::new();
    
    for entry in dictionary.keys()
    {
        let codepoints = codepoints(entry);
        for i in 1..codepoints.len()
        {
            let toinsert = codepoints[0..i].iter().collect();
            contains_longer.insert(toinsert);
        }
    }
    
    Ok(DartDict{dict: dictionary, contains_longer, left_contexts, right_contexts, feature_bytes})
}