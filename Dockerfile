# 基于官方评测环境镜像，保证环境一致
FROM docker.educg.net/cg/os-contest:20250516

# 设置工作目录为评测系统默认的工作目录（你本地代码会被clone到这里）
WORKDIR /workspace

# 环境变量：cargo 使用本地vendor目录，不用联网下载
ENV CARGO_HOME=/workspace/.cargo
ENV RUSTUP_HOME=/workspace/.rustup

# 如果需要（确保 vendor 目录在 /workspace/vendor）
# 在项目根目录放置 .cargo/config.toml，内容示例：
# [source.crates-io]
# replace-with = "vendored-sources"
# [source.vendored-sources]
# directory = "vendor"

# 你可以在这里执行任何项目构建前的准备命令（可选）
# 例如安装额外依赖或调整权限

# 默认执行编译命令，和你Makefile里的 all 对应
CMD ["make", "all"]