use std::{net::{TcpListener, TcpStream, SocketAddr, IpAddr, Ipv4Addr}, thread, io::{Read, Write}, str, fs::{self, File}, sync::{Arc, Mutex}, path::{PathBuf}, collections::HashMap, time::{Duration, Instant}};
use anyhow::Result;
use std::env;
use crossbeam::atomic::AtomicCell;


#[derive(Clone)]
struct Commands {
    caching: bool,
    streaming: bool,
    blacklist: bool,
    cache_num_max: usize,
    requests_per_second_limit: i32,
    request_map: Arc<Mutex<HashMap<SocketAddr, i32>>>,
    the_blacklist: Arc<Mutex<Vec<SocketAddr>>>,
    cache_count_map: Arc<Mutex<HashMap<PathBuf, i32>>>,
    cache_data_map: Arc<Mutex<HashMap<PathBuf, String>>>,
    admin_ip: Vec<SocketAddr>,
}

fn main() -> std::io::Result<()>{
    let command: Vec<String> = std::env::args().skip(1).collect();
    let start = Instant::now();
    let mut commands = Commands {
        caching: false,
        streaming: false,
        blacklist: false,
        cache_num_max: 0,
        requests_per_second_limit: 25,
        request_map: Arc::new(Mutex::new(HashMap::new())),
        the_blacklist: Arc::new(Mutex::new(Vec::new())),
        cache_count_map: Arc::new(Mutex::new(HashMap::new())),
        cache_data_map: Arc::new(Mutex::new(HashMap::new())),
        admin_ip: Vec::new(),
    };
    commands.admin_ip.push(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8888));
    for c in command{
        if c.contains("-s"){
            commands.streaming = true;
        }
        if c.contains("-c"){
            commands.caching = true;
            let temp:Vec<&str> = c.split("=").collect();
            commands.cache_num_max = temp[temp.len()-1].parse().unwrap();
        }
        if c.contains("-b"){
            commands.blacklist = true;
        }
    }
    println!("Streaming: {}, Caching: {}, Blacklist: {}", commands.streaming, commands.caching, commands.blacklist);

    let listener = TcpListener::bind("localhost:8888")?;
    let total_requests = Arc::new(AtomicCell::new(0));
    let valid_requests = Arc::new(AtomicCell::new(0));
    for stream in listener.incoming() {
        let total_requests = total_requests.clone();
        let valid_requests = valid_requests.clone();
        let mut commands = commands.clone();
        if start.elapsed().as_secs() % 60 == 0{
            commands.request_map = Arc::new(Mutex::new(HashMap::new()));
        }
        thread::spawn(move || {
            match stream {
                Ok(match_stream) => {
                    let mut is_blacklisted = false;
                    let mut reason = "";
                    let mut is_admin = false;
                    let new_stream = match_stream;
                    for i in &commands.admin_ip{
                        if &new_stream.local_addr().unwrap() == i{
                            is_admin = true;
                        }
                    }
                    total_requests.fetch_update(|c| Some(c + 1)).unwrap();
                    println!("IP Address: {}", new_stream.local_addr().unwrap());
                    if commands.blacklist{
                        let mut map = commands.request_map.lock().unwrap();
                        if map.contains_key(&new_stream.local_addr().unwrap()){
                            *map.get_mut(&new_stream.local_addr().unwrap()).unwrap() += 1;
                        }
                        else{
                            map.insert(new_stream.local_addr().unwrap(), 1);
                        }

                        if map.get(&new_stream.local_addr().unwrap()).unwrap().to_owned() > commands.requests_per_second_limit * 10{
                            let mut blacklist = commands.the_blacklist.lock().unwrap();
                            if !blacklist.contains(&new_stream.local_addr().unwrap()){
                                blacklist.push(new_stream.local_addr().unwrap());
                            }
                            is_blacklisted = true;
                            reason = "Blacklisted";
                        }
                    }
                    if !is_blacklisted{
                        handle_client(new_stream, &valid_requests, commands, is_admin);
                    }
                    else{
                        println!("Client Refused for reason: {}", reason);
                    }
                    println!("Total Request Count: {}, Valid Request Count: {}\n", total_requests.load(), valid_requests.load());
                }
                Err(e) => {
                    println!("{e}");
                }
            }
        });
    }
    Ok(())
}

fn handle_client(mut stream: TcpStream, valid_requests: &Arc<AtomicCell<i32>>, commands: Commands, is_admin: bool) -> Result<()>{
    let mut received_message = String::new();
    while !received_message.contains("\r\n\r\n"){
        let mut buffer = [0;500];
        let size = stream.read(&mut buffer)?;
        let message = str::from_utf8(&buffer[0..size]).unwrap();
        received_message.push_str(message);
    }

    println!("Received Message: {received_message}");
    file_validation(received_message, valid_requests.to_owned(), stream, commands, is_admin);

    Ok(())
}

fn file_validation(message: String, valid_requests: Arc<AtomicCell<i32>>, mut stream: TcpStream, commands: Commands, is_admin: bool) -> Result<()>{
    let split:Vec<&str> = message.split("\n").collect();
    let split_line:Vec<&str> = split[0].split(" ").collect();
    let mut get = split_line[1].to_string();
    get.remove(0);
    
    let mut working_directory = env::current_dir()?;
    working_directory.push("files");
    working_directory.push(&get);
    let mut absolute_path = PathBuf::new();

    //create an absolute path out of the relative path to ensure it's a subdirectory, and if the absolute path does not exist we return 404
    match fs::canonicalize(&working_directory){
        Ok(new_path) => {
            absolute_path = new_path;
        }
        Err(e) => {
            println!("{}", e);
        }
    };

    //if absolute path is a directory not file then it is a 404 error
    if absolute_path.is_dir() || absolute_path.to_str().unwrap().is_empty(){
        println!("404 Error");
        let path = PathBuf::from("./files/404");
        write(path, stream);
    } 
    //if absolute path doesnt start with the working directory and the absolute path is not empty then it is a 403 error
    else if !absolute_path.to_str().unwrap().starts_with(working_directory.to_str().unwrap()) && !absolute_path.to_str().unwrap().is_empty(){
        println!("403 Error");
        let path = PathBuf::from("./files/403");
        write(path, stream);
    }
    else if get.contains("ADMIN") && !is_admin{
        println!("401 Error");
        let path = PathBuf::from("./files/401");
        write(path, stream);
    }
    //if absolute path is not empty at this point then we know the path is valid and we can send the data
    else if !absolute_path.to_str().unwrap().is_empty(){
        //add 1 to valid request and process caching and streaming
        valid_requests.fetch_update(|c| Some(c + 1)).unwrap();
        //if we are caching files we use the file from our 
        if commands.caching{
            let file_message = cache_message(commands, absolute_path.to_owned());
            if message.is_empty(){
                let mut header = "HTTP/1.1 200 OK\nContent-Type: text/html; charset=UTF-8\nContent-Length: ".to_string();
                header.push_str(format!("{}\r\n\r\n", file_message.len()).as_str());
                let send_message = header + file_message.as_str();
                stream.write(send_message.as_bytes())?;
            }
            else{
                write(absolute_path, stream);
            }
        }
        else if commands.streaming{
            let header = "HTTP/1.1 200 OK\nContent-Type: text/html; charset=UTF-8\nContent-Length: ".to_string();
            stream_message(header, absolute_path, stream);
        }
        else{
            write(absolute_path, stream);
        }
    }

    println!("GET: {}", get);
    Ok(())
}

//read 500 bytes from a file and write it to the stream at a time
fn stream_message(mut header: String, path: PathBuf, mut stream: TcpStream) -> Result<()>{
    header.push_str(format!("{}\r\n\r\n", fs::metadata(&path)?.len()).as_str());
    stream.write(header.as_bytes())?;
    let mut f = File::open(&path)?;
    let mut size:usize = 1;
    let mut buffer = [0;500];
    while size != 0{
        size = f.read(&mut buffer)?;
        let message = str::from_utf8(&buffer[0..size]).unwrap();
        stream.write(message.as_bytes())?;
    }

    Ok(())
}

fn cache_message(commands: Commands, absolute_path: PathBuf) -> String{
    //lock the map and clone it to a new variable to allow for immediate release
    let mut cache_count_map = HashMap::new();
    {
        let mut count_map = commands.cache_count_map.lock().unwrap();
        //insert if map doesnt contain file, add one to count otherwise
        if !count_map.contains_key(&absolute_path){
            count_map.insert(absolute_path.to_owned(), 1);
        }
        else{
            *count_map.get_mut(&absolute_path).unwrap() += 1;
        }
        cache_count_map = count_map.clone();
    }

    //sort the map from highest to lowest count and remove all but the cache_num_max top paths
    let mut sorted_map: Vec<(&PathBuf, &i32)> = cache_count_map.iter().collect();
    sorted_map.sort_by(|a, b| b.1.cmp(a.1));
    while sorted_map.len() > commands.cache_num_max{
        sorted_map.remove(sorted_map.len()-1);
    }

    //lock the map and clone it to a new variable to allow for immediate release
    let mut cache_file_map = HashMap::new();
    {
        let mut file_map = commands.cache_data_map.lock().unwrap();
        //walk through sorted top paths and if the value is not 0 and cache_file does not already contain the path then insert the file contents to the map
        for i in &sorted_map{
            if !file_map.contains_key(i.0) && i.1 != &0{
                let temp_file = fs::read_to_string(i.0).unwrap();
                file_map.insert(i.0.to_owned(), temp_file);
            }
        }
        //walk through cache_file and if sortmap does not contain the path then push the path to remove_keys
        let mut remove_keys:Vec<PathBuf> = Vec::new();
        for i in file_map.keys(){
            if !sorted_map.contains(&(i, cache_count_map.get(i).unwrap())){
                remove_keys.push(i.to_owned());
            }
        }
        //remove all paths found in remove_keys from cache_file because they are no longer within cache_num in terms of popularity
        for i in remove_keys{
            file_map.remove(&i);
        }
        cache_file_map = file_map.clone();
    }

    for i in cache_count_map{
        println!("File Path: {}, Times Accessed: {}", i.0.as_os_str().to_str().unwrap(), i.1);
    }

    if cache_file_map.contains_key(&absolute_path){
        return cache_file_map.get(&absolute_path).unwrap().to_owned();
    }
    else{
        return "".to_string();
    }
}

fn write(absolute_path: PathBuf, mut stream: TcpStream) -> Result<()>{
    let mut header = "HTTP/1.1 200 OK\nContent-Type: text/html; charset=UTF-8\nContent-Length: ".to_string();
    let file_message = fs::read_to_string(absolute_path)?;
    header.push_str(format!("{}\r\n\r\n", file_message.len()).as_str());
    let send_message = header + file_message.as_str();
    stream.write(send_message.as_bytes())?;
    Ok(())
}