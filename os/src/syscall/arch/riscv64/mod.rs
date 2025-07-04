
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

pub const SYSCALL_NANOSLEEP :usize =101;
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
pub const SYSCALL_SIGTIMEDWAIT :usize= 137;
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
pub const SYSCALL_RENAMEAT: usize = 38;

pub const SYSCALL_RENAMEAT2: usize = 276;
// pub const SYSCALL_CREAT: usize = 85;
// pub const SYSCALL_RMDIR: usize = 84;
pub const SYSCALL_GETRANDOM: usize = 278;
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
pub const SYSCALL_CLOCK_GETTIME:usize = 113;
pub const SYSCALL_CLOCK_SETTIME:usize = 112;
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

pub const SYSCALL_MPROTECT:usize =226; 

pub const SYSCALL_PIPE2:usize =59; 
pub const SYSCALL_SENDFILE: usize= 71;
pub const SYSCALL_STATFS: usize= 43;
pub const SYSCALL_LOG:usize =116;
pub const SYSCALL_INFO:usize =179;

pub const SYSCALL_UTIMENSAT:usize =88;
pub const SYSCALL_FUTEX:usize = 98;


pub const SYSCALL_SOCKET:usize =198;
pub const SYSCALL_BIND:usize=200;

pub const SYSCALL_GETSOCKNAME:usize=204;

pub const SYSCALL_GETPEERNAME:usize=205;

pub const SYSCALL_SETSOCKOPT:usize=208;

pub const SYSCALL_SENDTO:usize=206;
pub const SYSCALL_RECVFROM :usize =207;

pub const SYSCALL_SENDMSG: usize = 211;
pub const SYSCALL_LISTEN: usize = 201;
pub const SYSCALL_ACCEPT: usize = 202;

pub const SYSCALL_ACCEPT4: usize = 242;
pub const SYSCALL_CONNECT: usize = 203;
pub const SYSCALL_SOCKETPAIR: usize = 199;
pub const SYSCALL_MREMAP: usize= 216;

pub const SYSCALL_SETSID :usize =157;
pub const SYSCALL_SCHED_YIELD :usize =124;
pub const SYSCALL_SETUID:usize = 146;
pub const SYSCALL_GETGID:usize = 176;

pub const SYSCALL_GETEGID:usize =177;
pub const SYSCALL_MEMBARRIER :usize =283;
pub const SYSCALL_SCHED_SETAFFINITY: usize = 122;
pub const SYSCALL_SCHED_GETAFFINITY: usize = 123;
pub const SYSCALL_MADVISE:usize = 233;
pub const SYSCALL_GET_MEMPOLICY:  usize= 236;
pub const SYSCALL_SET_MEMPOLICY: usize = 237;
pub const SYSCALL_SCHED_SETSCHEDULER:usize = 119;
pub const SYSCALL_SCHED_GETSCHEDULER:usize = 120;
pub const SYSCALL_SCHED_SETPARAM: usize = 118;
pub const SYSCALL_SCHED_GETPARAM: usize = 121;
pub const SYSCALL_TRUNCATE: usize = 45;
pub const SYSCALL_FTRUNCATE: usize = 46;
pub const SYSCALL_MLOCK: usize= 228;
pub const SYSCALL_MUNLOCK: usize= 229;
pub const SYSCALL_MLOCKALL: usize= 230;
pub const SYSCALL_MUNLOCKALL: usize= 231;
pub const SYSCALL_GETRUSAGE: usize = 165;
pub const SYSCALL_PSELECT6: usize = 72;
pub const SYSCALL_SYNC: usize= 81;
pub const SYSCALL_FSYNC: usize = 82;




//todo();
pub const SYSCALL_GETSOCKOPT: usize = 209;
pub const SYSCALL_SHMGET: usize= 194;
pub const SYSCALL_SHMAT: usize= 196;
pub const SYSCALL_SHMCTL: usize= 195;
pub const SYSCALL_SHMDT: usize= 197;
pub const SYSCALL_GETITIMER: usize = 102;
pub const SYSCALL_SETITIMER: usize = 103;
pub const SYSCALL_UMASK:usize = 166;
