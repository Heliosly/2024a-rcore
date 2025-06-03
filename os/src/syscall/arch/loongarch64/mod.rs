
/// unlinkat syscall
const SYSCALL_UNLINKAT: usize = 35;
/// linkat syscall
const SYSCALL_LINKAT: usize = 37;
/// open syscall
const SYSCALL_OPEN: usize = 56;
/// close syscall
const SYSCALL_CLOSE: usize = 57;
/// read syscall
const SYSCALL_READ: usize = 63;
/// write syscall
const SYSCALL_WRITE: usize = 64;
/// fstat syscall
const SYSCALL_FSTAT: usize = 80;
/// exit syscall
const SYSCALL_EXIT: usize = 93;
/// yield syscall
const SYSCALL_YIELD: usize = 124;

const SYSCALL_KILL: usize = 129;
/// kill syscall
const SYSCALL_TKILL: usize = 130;
/// tkill syscall
const SYSCALL_TGKILL: usize = 131;
/// setpriority syscall
const SYSCALL_SET_PRIORITY: usize = 140;
/// gettime syscall
const SYSCALL_GET_TIME: usize = 169;
/// getpid syscall
const SYSCALL_GETPID: usize = 172;
/// sbrk syscall
const SYSCALL_BRK: usize = 214;
/// munmap syscall
const SYSCALL_MUNMAP: usize = 215;
/// fork syscall
// const SYSCALL_FORK: usize = 220;
/// exec syscall
const SYSCALL_EXEC: usize = 221;
/// mmap syscall
const SYSCALL_MMAP: usize = 222;
/// waitpid syscall
const SYSCALL_WAITPID: usize = 260;
/// spawn syscall
const SYSCALL_SPAWN: usize = 400;
/// taskinfo syscall
const SYSCALL_TASK_INFO: usize = 410;
/// clone 
const SYSCALL_CLONE:usize = 220;
///set tid address
const SYSCALL_SETTIDADDRESS :usize =96;
///get uid
const SYSCALL_GETUID :usize = 174;
///exit group
const SYSCALL_EXITGROUP :usize=  94;
///
const SYSCALL_LSEEK: usize = 62;
const SYSCALL_READV: usize = 65;
const SYSCALL_WRITEV: usize = 66;
const SYSCALL_PREAD64: usize = 67;
const SYSCALL_PWRITE64: usize = 68;
const SYSCALL_RENAMEAT: usize = 38;
const SYSCALL_SIGPROCMASK :usize =135;
const SYSCALL_RT_SIGACTION :usize =134;
const SYSCALL_GETPPID:usize = 173;
const SYSCALL_UNAME:usize = 160;
const SYSCALL_FSTATAT :usize =79;
const SYSCALL_IOCTL :usize =29;
const SYSCALL_FCNTL:usize =25;
const SYSCALL_SIGNALRET:usize =139;
const SYSCALL_GETEUID:usize=175;
const SYSCALL_GETCWD:usize= 17;
const SYSCALL_PPOLL:usize = 73;
const SYSCALL_CHDIR:usize = 49;
const SYSCALL_GETDENTS64:usize=61;
const SYSCALL_GETPGID :usize = 155;
const SYSCALL_SETPGID :usize = 154;
const SYSCALL_CLOCK_GETTIME:usize = 112;
const SYSCALL_CLOCK_SETTIME:usize = 113;
const SYSCALL_CLOCK_GETRES:usize = 114; 
const SYSCALL_GETTID:usize=178;
const SYSCALL_FACCESSAT:usize=48;
const SYSCALL_SETROBUSTLIST :usize =99;
const SYSCALL_MKDIRAT:usize =34;
const SYSCALL_GETROBUSTLIST :usize =100;
const SYSCALL_DUP2:usize=23;

const SYSCALL_DUP3:usize=24;