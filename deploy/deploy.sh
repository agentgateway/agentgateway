#!/bin/bash
# ===========================================================================
# UnitOne AgentGateway - Azure Deployment Script
# Based on MCP Scanner deployment pattern
# ===========================================================================

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check prerequisites
check_prerequisites() {
    log_info "Checking prerequisites..."

    if ! command -v az &> /dev/null; then
        log_error "Azure CLI is not installed. Please install it first."
        exit 1
    fi

    if ! az account show &> /dev/null; then
        log_error "Not logged in to Azure. Please run 'az login' first."
        exit 1
    fi

    log_success "Prerequisites check passed"
}

# Parse arguments
ENVIRONMENT="dev"
BUILD_IMAGE=false
IMAGE_TAG="latest"
SUBSCRIPTION_ID=""

while [[ $# -gt 0 ]]; do
    case $1 in
        -e|--environment)
            ENVIRONMENT="$2"
            shift 2
            ;;
        -b|--build)
            BUILD_IMAGE=true
            shift
            ;;
        -t|--tag)
            IMAGE_TAG="$2"
            shift 2
            ;;
        -s|--subscription)
            SUBSCRIPTION_ID="$2"
            shift 2
            ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo "Options:"
            echo "  -e, --environment ENV    Environment to deploy (dev, staging, prod) [default: dev]"
            echo "  -b, --build              Build and push Docker image before deploying"
            echo "  -t, --tag TAG            Docker image tag [default: latest]"
            echo "  -s, --subscription ID    Azure subscription ID"
            echo "  -h, --help               Show this help message"
            exit 0
            ;;
        *)
            log_error "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Validate environment
if [[ ! "$ENVIRONMENT" =~ ^(dev|staging|prod)$ ]]; then
    log_error "Invalid environment: $ENVIRONMENT. Must be dev, staging, or prod."
    exit 1
fi

# Set Azure subscription
if [ -n "$SUBSCRIPTION_ID" ]; then
    log_info "Setting Azure subscription to $SUBSCRIPTION_ID..."
    az account set --subscription "$SUBSCRIPTION_ID"
fi

CURRENT_SUBSCRIPTION=$(az account show --query id -o tsv)
log_info "Using Azure subscription: $CURRENT_SUBSCRIPTION"

# Variables
RESOURCE_GROUP="unitone-agw-${ENVIRONMENT}-rg"
LOCATION="eastus2"
ACR_NAME="unitoneagw${ENVIRONMENT}acr"
IMAGE_NAME="unitone-agentgateway"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

log_info "====================================="
log_info "Deployment Configuration"
log_info "====================================="
log_info "Environment: $ENVIRONMENT"
log_info "Resource Group: $RESOURCE_GROUP"
log_info "Location: $LOCATION"
log_info "ACR: $ACR_NAME"
log_info "Image: $IMAGE_NAME:$IMAGE_TAG"
log_info "Build Image: $BUILD_IMAGE"
log_info "====================================="

# Check prerequisites
check_prerequisites

# Create resource group if it doesn't exist
log_info "Ensuring resource group exists..."
if ! az group show --name "$RESOURCE_GROUP" &> /dev/null; then
    log_info "Creating resource group $RESOURCE_GROUP in $LOCATION..."
    az group create --name "$RESOURCE_GROUP" --location "$LOCATION"
    log_success "Resource group created"
else
    log_success "Resource group already exists"
fi

# Build and push Docker image if requested
if [ "$BUILD_IMAGE" = true ]; then
    log_info "Building Docker image..."

    # Get ACR login server (will be created by Bicep if doesn't exist)
    ACR_LOGIN_SERVER="${ACR_NAME}.azurecr.io"

    log_info "Building image using Azure Container Registry..."
    az acr build \
        --registry "$ACR_NAME" \
        --image "${IMAGE_NAME}:${IMAGE_TAG}" \
        --image "${IMAGE_NAME}:$(date +%Y%m%d-%H%M%S)" \
        --file "$REPO_ROOT/Dockerfile.acr" \
        --platform linux/amd64 \
        "$REPO_ROOT" || {
            log_error "Docker build failed"
            exit 1
        }

    log_success "Docker image built and pushed to $ACR_LOGIN_SERVER/${IMAGE_NAME}:${IMAGE_TAG}"
fi

# Deploy infrastructure with Bicep
log_info "Deploying infrastructure with Bicep..."

PARAMETERS_FILE="$REPO_ROOT/deploy/bicep/parameters-${ENVIRONMENT}.json"

if [ ! -f "$PARAMETERS_FILE" ]; then
    log_error "Parameters file not found: $PARAMETERS_FILE"
    exit 1
fi

# Update parameter file with current subscription ID
log_info "Updating parameter file with subscription ID..."
sed -i.bak "s|<SUBSCRIPTION_ID>|$CURRENT_SUBSCRIPTION|g" "$PARAMETERS_FILE"

az deployment group create \
    --resource-group "$RESOURCE_GROUP" \
    --template-file "$REPO_ROOT/deploy/bicep/main.bicep" \
    --parameters "@$PARAMETERS_FILE" \
    --parameters imageTag="$IMAGE_TAG" \
    --name "agw-deployment-$(date +%Y%m%d-%H%M%S)" \
    --verbose || {
        log_error "Bicep deployment failed"
        # Restore backup
        mv "$PARAMETERS_FILE.bak" "$PARAMETERS_FILE"
        exit 1
    }

# Restore backup
mv "$PARAMETERS_FILE.bak" "$PARAMETERS_FILE"

log_success "Infrastructure deployed successfully"

# Get deployment outputs
log_info "Retrieving deployment outputs..."
CONTAINER_APP_URL=$(az deployment group show \
    --resource-group "$RESOURCE_GROUP" \
    --name "agw-deployment-$(date +%Y%m%d-%H%M%S)" \
    --query "properties.outputs.containerAppUrl.value" \
    -o tsv 2>/dev/null || echo "")

if [ -n "$CONTAINER_APP_URL" ]; then
    log_success "====================================="
    log_success "Deployment Complete!"
    log_success "====================================="
    log_success "UI URL: ${CONTAINER_APP_URL}/ui"
    log_success "MCP Endpoint: ${CONTAINER_APP_URL}/mcp"
    log_success "====================================="
else
    log_warn "Could not retrieve Container App URL. Check Azure Portal."
fi

log_info "To view logs:"
log_info "  az containerapp logs show --name unitone-agw-${ENVIRONMENT}-app --resource-group $RESOURCE_GROUP --follow"

log_info "To update image:"
log_info "  ./deploy.sh --environment $ENVIRONMENT --build --tag $IMAGE_TAG"
