# Dockerfile for testing E2E setup infrastructure
# This simulates a new developer environment to validate our improvements

FROM ubuntu:22.04

# Prevent interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=UTC

# Install basic system dependencies that a developer machine would have
RUN apt-get update && apt-get install -y \
    curl \
    wget \
    git \
    build-essential \
    pkg-config \
    ca-certificates \
    lsof \
    procps \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js (required for resource detection script)
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
    apt-get install -y nodejs

# Create a non-root user to simulate developer environment
RUN useradd -m -s /bin/bash developer && \
    echo "developer ALL=(ALL) NOPASSWD:ALL" >> /etc/sudoers

# Switch to developer user
USER developer
WORKDIR /home/developer

# Set up environment variables
ENV HOME=/home/developer
ENV PATH=$HOME/.cargo/bin:$PATH

# Copy the project files (this simulates git clone)
COPY --chown=developer:developer . /home/developer/agentgateway

# Copy the validation script
COPY --chown=developer:developer docker/validate-e2e-setup.sh /home/developer/validate-e2e-setup.sh

# Make the validation script executable
RUN chmod +x /home/developer/validate-e2e-setup.sh

# Set working directory to the project
WORKDIR /home/developer/agentgateway

# Set the default command to run our validation
CMD ["/home/developer/validate-e2e-setup.sh"]
