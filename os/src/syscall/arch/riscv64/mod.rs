
/// unlinkat syscall
pub const SYSCALL_UNLINKAT: usize = 35;
/// linkat syscall
pub const SYSCALL_LINKAT: usize = 37;
/// open syscall
pub const SYSCALL_OPEN: usize = 56;
/// close syscall
pub const SYSCALL_CLOSE: usize = 57;
/// read syscall
pub const SYSCALL_READ: usize = 63;
/// write syscall
pub const SYSCALL_WRITE: usize = 64;
/// fstat syscall
pub const SYSCALL_FSTAT: usize = 80;
/// exit syscall
pub const SYSCALL_EXIT: usize = 93;
/// yield syscall
pub const SYSCALL_YIELD: usize = 124;

pub const SYSCALL_KILL: usize = 129;
/// kill syscall
pub const SYSCALL_TKILL: usize = 130;
/// tkill syscall
pub const SYSCALL_TGKILL: usize = 131;
/// setpriority syscall
pub const SYSCALL_SET_PRIORITY: usize = 140;
/// getpid syscall
pub const SYSCALL_GETPID: usize = 172;
pub const SYSCALL_CLOCK_NANOSLEEP :usize =115;
/// sbrk syscall
pub const SYSCALL_BRK: usize = 214;
/// munmap syscall
pub const SYSCALL_MUNMAP: usize = 215;
/// fork syscall
// pub const SYSCALL_FORK: usize = 220;
/// exec syscall
pub const SYSCALL_EXEC: usize = 221;
/// mmap syscall
pub const SYSCALL_MMAP: usize = 222;
/// waitpid syscall
pub const SYSCALL_WAITPID: usize = 260;
/// spawn syscall
pub const SYSCALL_SPAWN: usize = 400;
/// taskinfo syscall
pub const SYSCALL_TASK_INFO: usize = 410;
/// clone 
pub const SYSCALL_CLONE:usize = 220;
///set tid address
pub const SYSCALL_SETTIDADDRESS :usize =96;
///get uid
pub const SYSCALL_GETUID :usize = 174;
///exit group
pub const SYSCALL_EXITGROUP :usize=  94;
///
pub const SYSCALL_TIMES : usize =153;
pub const SYSCALL_GETTIMEOFDAY :usize =169;
pub const SYSCALL_LSEEK: usize = 62;
pub const SYSCALL_READV: usize = 65;
pub const SYSCALL_WRITEV: usize = 66;
pub const SYSCALL_PREAD64: usize = 67;
pub const SYSCALL_PWRITE64: usize = 68;
pub const SYSCALL_SIGPROCMASK :usize =135;
pub const SYSCALL_RT_SIGACTION :usize =134;
pub const SYSCALL_GETPPID:usize = 173;
pub const SYSCALL_UNAME:usize = 160;
pub const SYSCALL_FSTATAT :usize =79;
pub const SYSCALL_IOCTL :usize =29;
pub const SYSCALL_FCNTL:usize =25;
pub const SYSCALL_SIGNALRET:usize =139;
pub const SYSCALL_GETEUID:usize=175;
pub const SYSCALL_GETCWD:usize= 17;
pub const SYSCALL_PPOLL:usize = 73;
pub const SYSCALL_CHDIR:usize = 49;
pub const SYSCALL_GETDENTS64:usize=61;
pub const SYSCALL_GETPGID :usize = 155;
pub const SYSCALL_SETPGID :usize = 154;
pub const SYSCALL_CLOCK_GETTIME:usize = 112;
pub const SYSCALL_CLOCK_SETTIME:usize = 113;
pub const SYSCALL_CLOCK_GETRES:usize = 114; 
pub const SYSCALL_GETTID:usize=178;
pub const SYSCALL_FACCESSAT:usize=48;
pub const SYSCALL_SETROBUSTLIST :usize =99;
pub const SYSCALL_MKDIRAT:usize =34;
pub const SYSCALL_GETROBUSTLIST :usize =100;
pub const SYSCALL_DUP2:usize=23;

pub const SYSCALL_DUP3:usize=24;
pub const SYSCALL_PRLIMIT64 :usize =261;
pub const SYSCALL_MOUNT:usize=40;
pub const SYSCALL_UMOUNT2:usize=39;
pub const SYSCALL_SYMLINKAT:usize =36;
pub const SYSCALL_READLINKAT:usize =78;
pub const SYSCALL_GETRANDOM:usize =278; 

pub const SYSCALL_MPROTECT:usize =226; 

pub const SYSCALL_PIPE2:usize =59; 
pub const SYSCALL_SENDFILE: usize= 71;
pub const SYSCALL_STATFS: usize= 43;
pub const SYSCALL_LOG:usize =116;
pub const SYSCALL_INFO:usize =179;

pub const SYSCALL_UTIMENSAT:usize =88;