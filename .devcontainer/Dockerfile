# .devcontainer/Dockerfile
FROM node:20-bullseye

# Install system dependencies
RUN apt-get update && apt-get install -y \
    git \
    curl \
    vim \
    zsh \
    postgresql-client \
    build-essential \
    python3 \
    && rm -rf /var/lib/apt/lists/*

# Install pnpm
RUN npm install -g pnpm@10.13.1

# Install oh-my-zsh for better terminal experience
RUN sh -c "$(curl -fsSL https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)" "" --unattended

# Set zsh as default shell
RUN chsh -s $(which zsh) node

# Create workspace directory
WORKDIR /workspace

# Set proper permissions
RUN chown -R node:node /workspace

USER node
