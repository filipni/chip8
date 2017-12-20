extern crate rand;
extern crate rustbox;

use rand::Rng;

use rustbox::{Color, RustBox};
use rustbox::Event::KeyEvent;
use rustbox::Key;

use std::thread;
use std::fs::File;
use std::time::Duration;
use std::time::Instant;
use std::default::Default;
use std::io::prelude::*;

const PROC_FREQ_HZ: u64 = 500;
const CYCLE_TIME_MS: u64 = 1000 / PROC_FREQ_HZ;

const RAM_SIZE: usize = 0x1000 * 2;  
const STACK_SIZE: usize = 16;
const NUM_REGISTERS: usize = 16;

const DISP_WIDTH: usize = 64;
const DISP_HEIGHT: usize = 32;

const PROGRAM_START: usize = 0x200; 
const ROM_NAME: &'static str = "roms/breakout.rom";
const TIMER_DELAY_MS: u64 = 17;

const FONTS: [u8; 80] = [
    0xF0,0x90,0x90,0x90,0xF0, // 0
    0x20,0x60,0x20,0x20,0x70, // 1
    0xF0,0x10,0xF0,0x80,0xF0, // 2
    0xF0,0x10,0xF0,0x10,0xF0, // 3
    0x90,0x90,0xF0,0x10,0x10, // 4
    0xF0,0x80,0xF0,0x10,0xF0, // 5
    0xF0,0x80,0xF0,0x90,0xF0, // 6
    0xF0,0x10,0x20,0x40,0x40, // 7
    0xF0,0x90,0xF0,0x90,0xF0, // 8
    0xF0,0x90,0xF0,0x10,0xF0, // 9
    0xF0,0x90,0xF0,0x90,0x90, // A
    0xE0,0x90,0xE0,0x90,0xE0, // B
    0xF0,0x80,0x80,0x80,0xF0, // C
    0xE0,0x90,0x90,0x90,0xE0, // D
    0xF0,0x80,0xF0,0x80,0xF0, // E
    0xF0,0x80,0xF0,0x80,0x80  // F
];

fn convert_to_16bit (high_bits: &u8, low_bits: &u8) -> u16 {
    (*high_bits as u16) << 8 | *low_bits as u16
}

fn get_nibble(bits: &u16) -> u16 {
    *bits & 0x000F
}

fn get_byte(bits: &u16) -> u16 {
    *bits & 0x00FF
}

fn get_reg_x(op: &u16) -> usize {
    get_nibble(&(*op >> 8)) as usize 
}

fn get_reg_y(op: &u16) -> usize {
    get_nibble(&(*op >> 4)) as usize 
}

fn get_byte_value(op: &u16) -> u8 {
    get_byte(op) as u8
}

fn get_jump_addr(op: &u16) -> usize {
    (*op & 0x0FFF) as usize
}

struct Core {
    memory: [u8; RAM_SIZE],
    stack: [usize; STACK_SIZE],

    registers: [u8; NUM_REGISTERS],
    pc: usize,
    sp: usize,
    i_register: usize,

    inputs: [u8; NUM_REGISTERS],
    display: [[u8; DISP_WIDTH]; DISP_HEIGHT],
    update_display: bool,

    delay_timer: u8,
    sound_timer: u8,
    timer_60_hz: Instant,
}

impl Core {
    fn new() -> Core {
        Core {  
                memory: [0; RAM_SIZE], 
                stack: [0; STACK_SIZE],
                registers: [0; NUM_REGISTERS], 
                pc: PROGRAM_START, 
                sp: 0, 
                i_register: 0, 
                inputs: [0; NUM_REGISTERS],
                display: [[0; DISP_WIDTH]; DISP_HEIGHT],
                update_display: false,
                timer_60_hz: Instant::now(), 
                delay_timer: 0, 
                sound_timer: 0, 
        } 
    }

    fn read_program(&mut self, rom_name: &str) {
        let file = File::open(rom_name).expect("File not found.");
        for (i, byte) in file.bytes().enumerate() {
           self.memory[PROGRAM_START + i] = byte.unwrap(); 
        }
    }

    fn read_font(&mut self) {
        for (i, byte) in FONTS.iter().enumerate() {
            self.memory[i] = *byte;
        }
    }

    fn print_display(&self, rustbox: &RustBox) {
        for i in 0..DISP_HEIGHT {
            for j in 0..DISP_WIDTH {
                let pixel = self.display[i][j];
                if pixel == 1 {
                    rustbox.print(j, i, rustbox::RB_BOLD, Color::White, Color::White, "\u{2588}");
                } else {
                    rustbox.print(j, i, rustbox::RB_BOLD, Color::Black, Color::Black, "\u{2588}");
                }
            }
        }
        rustbox.present();
    }

    fn run_next(&mut self) {
        let op: u16 = convert_to_16bit(&self.memory[self.pc], &self.memory[self.pc + 1]);
        match op {
            0x0000 ... 0x0FFF => {
                match get_byte(&op) {
                   0xE0 => self.clear_screen(),
                   0xEE => self.ret(),
                   _    => self.not_implemented(op),
                   // Ignore instruction 0nnn - SYS addr
                }
            },
            0x1000 ... 0x1FFF => {
                self.jump(op)
            },
            0x2000 ... 0x2FFF => {
                self.call(op) 
            },
            0x3000 ... 0x3FFF => {
                self.inc_pc_eq(op)
            },
            0x4000 ... 0x4FFF => {
                self.inc_pc_ne(op)
            },
            0x5000 ... 0x5FF0 => {
                self.inc_pc_reg_eq(op)
            },
            0x6000 ... 0x6FFF => {
                self.set_register(op)    
            },
            0x7000 ... 0x7FFF => {
                self.add_val_to_reg(op)
            }
            0x8000 ... 0x8FF0 => {
                match get_nibble(&op) {
                    0x0 => self.set_reg_to_reg(op),
                    0x1 => self.or(op),
                    0x2 => self.and(op),
                    0x3 => self.xor(op),
                    0x4 => self.add(op),
                    0x5 => self.sub(op),
                    0x6 => self.shr(op),
                    0x7 => self.subn(op),
                    0xE => self.shl(op),
                    _   => self.not_implemented(op),
                }
            },
            0x9000 ... 0x9FF0 => {
                self.inc_pc_reg_ne(op)
            },
            0xA000 ... 0xAFFF => {
                self.load_reg_i(op)
            },
            0xC000 ... 0xCFFF => {
                self.get_rand_byte(op)
            },
            0xD000 ... 0xDFFF => {
                self.display_sprite(op)
            },
            0xE000 ... 0xEFFF => {
                match get_byte(&op) {
                    0x9E => self.skip_if_pressed(op),
                    0xA1 => self.skip_if_not_pressed(op),
                    _    => self.not_implemented(op),
                }
            },
            0xF000 ... 0xFFFF => {
                match get_byte(&op) {
                    0x07 => self.load_delay_timer(op),
                    0x33 => self.store_bcd(op),
                    0x15 => self.set_delay_timer(op),
                    0x18 => self.set_sound_timer(op),
                    0x1E => self.add_reg_to_i(op),
                    0x29 => self.set_digit_addr(op),
                    0x55 => self.store_registers(op),
                    0x65 => self.load_registers(op),
                    _    => self.not_implemented(op),
                }
            },
            _ => {
                self.not_implemented(op)
            }
        };
        
        if self.timer_60_hz.elapsed() >= Duration::from_millis(TIMER_DELAY_MS) {
            self.timer_60_hz = Instant::now();
            if self.delay_timer > 0 { self.delay_timer -= 1; } 
            if self.sound_timer > 0 { self.sound_timer -= 1; } // Make BEEP when sound_timer > 0 - Not implemented!
        }
    }

    fn shl(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        let val = self.registers[x_reg];

        self.registers[0xF] = (val & 80 != 0) as u8;
        self.registers[x_reg] <<= 1;

        self.inc_pc();
    }

    fn shr(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        let val = self.registers[x_reg];

        self.registers[0xF] = val & 1;
        self.registers[x_reg] >>= 1;

        self.inc_pc();
    }

    fn store_registers(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        for i in 0..x_reg+1 {
            self.memory[self.i_register + i] = self.registers[i];
        }
        self.inc_pc();
    }

    fn add_reg_to_i(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        self.i_register += self.registers[x_reg] as usize;
        self.inc_pc();
    }

    fn sub(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        let y_reg = get_reg_y(&op);
        let x_term = self.registers[x_reg] as i16; 
        let y_term  = self.registers[y_reg] as i16;

        self.registers[0xF] = (x_term > y_term) as u8;
        let sum: i16 = x_term - y_term;
        self.registers[x_reg] = (sum & 0xFF) as u8; 

        self.inc_pc();
    }

    fn subn(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        let y_reg = get_reg_y(&op);

        let x_term = self.registers[x_reg] as i16; 
        let y_term  = self.registers[y_reg] as i16;

        self.registers[0xF] = (y_term > x_term) as u8;
        let sum: i16 = y_term - x_term;
        self.registers[x_reg] = (sum & 0xFF) as u8; 

        self.inc_pc();
    }

    fn add(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        let y_reg = get_reg_y(&op);

        let sum: u16 = self.registers[x_reg] as u16 + self.registers[y_reg] as u16 ;
        if sum > 255 {
            self.registers[0xF] = 1;
        } else {
            self.registers[0xF] = 0;
        }
        self.registers[x_reg] = (sum & 0xFF) as u8; 

        self.inc_pc();
    }
    
    fn and(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        let y_reg = get_reg_y(&op);
        self.registers[x_reg] = self.registers[x_reg] & self.registers[y_reg];
        self.inc_pc();
    }

    fn or(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        let y_reg = get_reg_y(&op);
        self.registers[x_reg] = self.registers[x_reg] | self.registers[y_reg];
        self.inc_pc();
    }

    fn xor(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        let y_reg = get_reg_y(&op);
        self.registers[x_reg] = self.registers[x_reg] ^ self.registers[y_reg];
        self.inc_pc();
    }

    fn set_sound_timer(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        self.sound_timer = self.registers[x_reg];
        self.inc_pc();
    }

    fn skip_if_pressed(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        let input = self.inputs[self.registers[x_reg] as usize];
        
        if input == 1 { 
            self.inputs[self.registers[x_reg] as usize] = 0;
            self.inc_pc(); 
        }

        self.inc_pc();
    }

    fn skip_if_not_pressed(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        let input = self.inputs[self.registers[x_reg] as usize];
        
        if input == 0 {
            self.inc_pc();
        } else {
            self.inputs[self.registers[x_reg] as usize] = 0;
        }

        self.inc_pc();
    }

    fn get_rand_byte(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        let bit_mask = get_byte_value(&op);

        let mut rng = rand::thread_rng();
        let rand_val = rng.gen::<u8>();
        
        self.registers[x_reg] = rand_val & bit_mask; 

        self.inc_pc();
    }

    fn set_digit_addr(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        self.i_register = self.registers[x_reg] as usize * 5;
        self.inc_pc();
    }

    fn load_registers(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        for i in 0..x_reg+1 {
            self.registers[i] = self.memory[self.i_register + i];
        }
        self.inc_pc();
    }

    fn load_delay_timer(&mut self, op: u16) {
        let x_reg = get_reg_x(&op); 
        self.registers[x_reg] = self.delay_timer;
        self.inc_pc();
    }

    fn set_delay_timer(&mut self, op: u16) {
        let x_reg = get_reg_x(&op); 
        self.delay_timer = self.registers[x_reg];
        self.inc_pc();
    }

    fn store_bcd(&mut self, op: u16) {
        let x_reg = get_reg_x(&op);
        let value = self.registers[x_reg];

        let one_digit = value % 10;
        let tens_digit = value % 100 / 10;
        let hundreds_digit = value / 100;

        self.memory[self.i_register] = hundreds_digit;
        self.memory[self.i_register + 1] = tens_digit;
        self.memory[self.i_register + 2] = one_digit;

        self.inc_pc();
    }

    fn ret(&mut self) {
        self.pc = self.stack[self.sp] as usize;
        self.sp -= 1;
    }

    fn jump(&mut self, op: u16) {
        let addr = op & 0x0FFF;
        self.pc = addr as usize;
    }

    fn load_reg_i(&mut self, op: u16) {
        self.i_register = (op & 0x0FFF) as usize; 
        self.inc_pc();
    }

    fn call(&mut self, op: u16) {
       self.inc_pc();
       self.sp += 1;
       self.stack[self.sp] = self.pc;
       self.pc = get_jump_addr(&op);
    }

    fn inc_pc_reg_eq(&mut self, op: u16) {
        let reg_x = get_reg_x(&op);
        let reg_y = get_reg_y(&op);
        if self.registers[reg_x] == self.registers[reg_y] { self.inc_pc(); }
        self.inc_pc();
    }

    fn inc_pc_reg_ne(&mut self, op: u16) {
        let reg_x = get_reg_x(&op);
        let reg_y = get_reg_y(&op);

        if self.registers[reg_x] != self.registers[reg_y] { self.inc_pc(); }
        self.inc_pc();
    }

    fn inc_pc_eq(&mut self, op: u16) {
        let reg_x = get_reg_x(&op);
        let reg_val = self.registers[reg_x];
        let op_val = get_byte_value(&op);

        if reg_val == op_val { self.inc_pc(); }
        self.inc_pc();
    }

    fn inc_pc_ne(&mut self, op: u16) {
        let reg_x = get_reg_x(&op);
        let reg_val = self.registers[reg_x];
        let op_val = get_byte_value(&op);

        if reg_val != op_val { self.inc_pc(); }
        self.inc_pc();
    }

    fn clear_screen(&mut self) {
        self.display = [[0; DISP_WIDTH]; DISP_HEIGHT];
        self.inc_pc();
    }

    fn set_register(&mut self, op: u16) {
        let reg_x = get_reg_x(&op);
        let val = get_byte_value(&op);
        self.registers[reg_x] = val;
        self.inc_pc();
    }

    fn set_reg_to_reg(&mut self, op: u16) {
        let reg_x = get_reg_x(&op);
        let reg_y = get_reg_y(&op);
        self.registers[reg_x] = self.registers[reg_y]; 
        self.inc_pc();
    }

    fn display_sprite(&mut self, op: u16) {
        let reg_x = get_reg_x(&op);
        let reg_y = get_reg_y(&op);
        let x_start = self.registers[reg_x] as usize;
        let y_start = self.registers[reg_y] as usize;

        self.registers[0xF] = 0; // Reset collision flag
        let bytes = get_nibble(&op) as usize;

        for i in 0..bytes {
            let mem_byte = self.memory[self.i_register + i];
            for b in 0..8 {
                let old_bit = self.display[(y_start + i) % DISP_HEIGHT][(x_start + b) % DISP_WIDTH];
                let new_bit = (mem_byte >> (7-b) & 1) ^ old_bit;
                
                if old_bit == 1 && new_bit == 0 { 
                    self.registers[0xF] = 1; 
                } 
                
                self.display[(y_start + i) % DISP_HEIGHT][(x_start + b) % DISP_WIDTH] = new_bit;
            }
        }

        self.update_display = true;
        self.inc_pc();
    }

    fn add_val_to_reg(&mut self, op: u16) {
        let reg_x = get_reg_x(&op);
        let val = get_byte_value(&op) as u16;
        self.registers[reg_x] = (self.registers[reg_x] as u16 + val) as u8;
        self.inc_pc();
    }

    fn not_implemented(&self, op: u16) {
        panic!("Instruction 0x{:04x} is not implemented yet.", op)
    }

    fn inc_pc(&mut self) {
        self.pc += 2;
    }
}

fn init_rustbox() -> RustBox {
    match RustBox::init(Default::default()) {
        Result::Ok(v) => v,
        Result::Err(_) => panic!("Couldn't init rustbox"),
    }
}

fn read_input(processor: &mut Core, rustbox: &RustBox) -> bool {
    let key_event = rustbox.peek_event(Duration::from_millis(0), false).expect("Couldn't read input");
    match key_event {
        KeyEvent(key) => {
            match key {
                Key::Char('q') => return exit_program(rustbox),
                Key::Char('2') => processor.inputs[0x0] = 1,
                Key::Char('3') => processor.inputs[0x1] = 1, 
                Key::Char('4') => processor.inputs[0x2] = 1, 
                Key::Char('5') => processor.inputs[0x3] = 1, 
                Key::Char('w') => processor.inputs[0x4] = 1, 
                Key::Char('e') => processor.inputs[0x5] = 1, 
                Key::Char('r') => processor.inputs[0x6] = 1, 
                Key::Char('t') => processor.inputs[0x7] = 1, 
                Key::Char('s') => processor.inputs[0x8] = 1, 
                Key::Char('d') => processor.inputs[0x9] = 1, 
                Key::Char('f') => processor.inputs[0xA] = 1, 
                Key::Char('g') => processor.inputs[0xB] = 1, 
                Key::Char('x') => processor.inputs[0xC] = 1, 
                Key::Char('c') => processor.inputs[0xD] = 1, 
                Key::Char('v') => processor.inputs[0xE] = 1, 
                Key::Char('b') => processor.inputs[0xF] = 1, 
                _              => {},
            }
        }
        _ => {},
    }
    true 
}

fn exit_program(rustbox: &RustBox) -> bool {
    rustbox.print(0, DISP_HEIGHT, rustbox::RB_BOLD, Color::Blue, Color::Default, "q pressed, exiting program...");
    rustbox.present();
    thread::sleep(Duration::from_secs(1));
    false 
}

fn main() {
    let mut processor = Core::new();
    processor.read_font();
    processor.read_program(ROM_NAME);

    let rustbox = init_rustbox(); 

    let mut running = true;
    while running {
        running = read_input(&mut processor, &rustbox);
        processor.run_next(); 
        if processor.update_display {
            processor.print_display(&rustbox);
            processor.update_display = false;
        }
        thread::sleep(Duration::from_millis(CYCLE_TIME_MS));
    }
}
