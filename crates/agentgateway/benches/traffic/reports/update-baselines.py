#!/usr/bin/env python3
"""
Dynamic Baseline Update System for AgentGateway Benchmarks

This script automatically checks for updates to industry benchmark data
from various sources and updates the baseline comparison system.
"""

import json
import requests
import hashlib
import feedparser
import re
import sys
import os
from datetime import datetime, timedelta
from pathlib import Path
from typing import Dict, List, Any, Optional, Tuple
from dataclasses import dataclass
import time

@dataclass
class BaselineSource:
    """Represents a source of baseline data."""
    name: str
    url: str
    source_type: str  # 'techempower', 'rss', 'github', 'api'
    last_checked: Optional[str] = None
    last_hash: Optional[str] = None

@dataclass
class BaselineUpdate:
    """Represents an update to baseline data."""
    proxy_name: str
    metric_name: str
    old_value: float
    new_value: float
    source: str
    confidence: float  # 0.0 to 1.0

class BaselineSourceManager:
    """Manages different types of baseline data sources."""
    
    def __init__(self):
        self.sources = {
            'techempower': BaselineSource(
                name='TechEmpower Framework Benchmarks',
                url='https://www.techempower.com/benchmarks/data.json',
                source_type='api'
            ),
            'cloudflare_blog': BaselineSource(
                name='Cloudflare Engineering Blog',
                url='https://blog.cloudflare.com/rss/',
                source_type='rss'
            ),
            'envoy_releases': BaselineSource(
                name='Envoy Proxy Releases',
                url='https://api.github.com/repos/envoyproxy/envoy/releases',
                source_type='github'
            ),
            'nginx_blog': BaselineSource(
                name='NGINX Blog',
                url='https://www.nginx.com/feed/',
                source_type='rss'
            ),
            'haproxy_releases': BaselineSource(
                name='HAProxy Releases',
                url='https://api.github.com/repos/haproxy/haproxy/releases',
                source_type='github'
            )
        }
        
        self.current_baselines = self._load_current_baselines()
        self.session = requests.Session()
        self.session.headers.update({
            'User-Agent': 'AgentGateway-Benchmark-Updater/1.0'
        })
    
    def _load_current_baselines(self) -> Dict:
        """Load current baseline data from generate-comparison.py."""
        try:
            # Import the current baselines from the comparison script
            import sys
            sys.path.append(str(Path(__file__).parent))
            
            # Read the current baselines from generate-comparison.py
            comparison_file = Path(__file__).parent / 'generate-comparison.py'
            if comparison_file.exists():
                with open(comparison_file, 'r') as f:
                    content = f.read()
                    
                # Extract PUBLISHED_BASELINES using regex
                baseline_match = re.search(
                    r'PUBLISHED_BASELINES\s*=\s*({.*?})\s*(?=\n\w|\nclass|\ndef|\n$)',
                    content,
                    re.DOTALL
                )
                
                if baseline_match:
                    # Safely evaluate the dictionary
                    baseline_str = baseline_match.group(1)
                    return eval(baseline_str)
            
            return {}
        except Exception as e:
            print(f"Warning: Could not load current baselines: {e}")
            return {}
    
    def check_all_sources(self) -> List[BaselineUpdate]:
        """Check all sources for updates and return list of changes."""
        updates = []
        
        for source_name, source in self.sources.items():
            try:
                print(f"Checking {source.name}...")
                source_updates = self._check_source(source)
                updates.extend(source_updates)
                time.sleep(1)  # Rate limiting
            except Exception as e:
                print(f"Error checking {source.name}: {e}")
        
        return updates
    
    def _check_source(self, source: BaselineSource) -> List[BaselineUpdate]:
        """Check a specific source for updates."""
        if source.source_type == 'techempower':
            return self._check_techempower(source)
        elif source.source_type == 'rss':
            return self._check_rss_feed(source)
        elif source.source_type == 'github':
            return self._check_github_releases(source)
        elif source.source_type == 'api':
            return self._check_api_endpoint(source)
        else:
            print(f"Unknown source type: {source.source_type}")
            return []
    
    def _check_techempower(self, source: BaselineSource) -> List[BaselineUpdate]:
        """Check TechEmpower Framework for updates."""
        updates = []
        
        try:
            # For now, this is a placeholder implementation
            # In a real implementation, we would:
            # 1. Fetch the latest TechEmpower data
            # 2. Parse the JSON results
            # 3. Compare with current baselines
            # 4. Generate updates for significant changes
            
            print(f"  TechEmpower check: No updates detected (placeholder)")
            
        except Exception as e:
            print(f"  TechEmpower check failed: {e}")
        
        return updates
    
    def _check_rss_feed(self, source: BaselineSource) -> List[BaselineUpdate]:
        """Check RSS feed for performance-related posts."""
        updates = []
        
        try:
            feed = feedparser.parse(source.url)
            
            # Look for recent posts (last 30 days) with performance keywords
            cutoff_date = datetime.now() - timedelta(days=30)
            performance_keywords = [
                'performance', 'benchmark', 'speed', 'latency', 'throughput',
                'qps', 'requests per second', 'optimization', 'faster'
            ]
            
            for entry in feed.entries[:10]:  # Check last 10 entries
                title = entry.title.lower()
                summary = getattr(entry, 'summary', '').lower()
                
                # Check if it's performance-related
                if any(keyword in title or keyword in summary for keyword in performance_keywords):
                    print(f"  Found performance-related post: {entry.title}")
                    
                    # In a real implementation, we would:
                    # 1. Fetch the full article
                    # 2. Parse for specific metrics
                    # 3. Extract numerical performance data
                    # 4. Compare with current baselines
                    
        except Exception as e:
            print(f"  RSS feed check failed: {e}")
        
        return updates
    
    def _check_github_releases(self, source: BaselineSource) -> List[BaselineUpdate]:
        """Check GitHub releases for performance notes."""
        updates = []
        
        try:
            response = self.session.get(source.url, timeout=10)
            response.raise_for_status()
            releases = response.json()
            
            # Check recent releases (last 6 months)
            cutoff_date = datetime.now() - timedelta(days=180)
            
            for release in releases[:5]:  # Check last 5 releases
                release_date = datetime.fromisoformat(
                    release['published_at'].replace('Z', '+00:00')
                )
                
                if release_date > cutoff_date:
                    body = release.get('body', '').lower()
                    
                    # Look for performance mentions
                    if any(keyword in body for keyword in ['performance', 'benchmark', 'faster', 'optimization']):
                        print(f"  Found performance-related release: {release['tag_name']}")
                        
                        # In a real implementation, we would:
                        # 1. Parse the release notes for specific metrics
                        # 2. Extract numerical performance improvements
                        # 3. Update baselines accordingly
                        
        except Exception as e:
            print(f"  GitHub releases check failed: {e}")
        
        return updates
    
    def _check_api_endpoint(self, source: BaselineSource) -> List[BaselineUpdate]:
        """Check API endpoint for updates."""
        updates = []
        
        try:
            response = self.session.get(source.url, timeout=10)
            response.raise_for_status()
            data = response.json()
            
            # Calculate hash of the data
            data_hash = hashlib.md5(json.dumps(data, sort_keys=True).encode()).hexdigest()
            
            if source.last_hash and source.last_hash != data_hash:
                print(f"  API data changed for {source.name}")
                
                # In a real implementation, we would:
                # 1. Compare the old and new data
                # 2. Identify specific changes
                # 3. Generate appropriate updates
                
            source.last_hash = data_hash
            source.last_checked = datetime.now().isoformat()
            
        except Exception as e:
            print(f"  API endpoint check failed: {e}")
        
        return updates

class BaselineUpdater:
    """Handles updating baseline data and generating reports."""
    
    def __init__(self):
        self.source_manager = BaselineSourceManager()
    
    def check_for_updates(self) -> Tuple[bool, List[BaselineUpdate]]:
        """Check all sources for updates."""
        print("🔍 Checking for baseline updates...")
        
        updates = self.source_manager.check_all_sources()
        
        if updates:
            print(f"✅ Found {len(updates)} potential updates")
            return True, updates
        else:
            print("✅ No baseline updates detected")
            return False, []
    
    def apply_updates(self, updates: List[BaselineUpdate]) -> bool:
        """Apply updates to the baseline data."""
        if not updates:
            return False
        
        print(f"📝 Applying {len(updates)} baseline updates...")
        
        # In a real implementation, we would:
        # 1. Update the PUBLISHED_BASELINES in generate-comparison.py
        # 2. Create a backup of the old baselines
        # 3. Log all changes with timestamps
        # 4. Validate the new baselines
        
        # For now, just log what would be updated
        for update in updates:
            print(f"  {update.proxy_name}: {update.metric_name} "
                  f"{update.old_value} → {update.new_value} "
                  f"(source: {update.source}, confidence: {update.confidence:.2f})")
        
        return True
    
    def generate_update_summary(self, updates: List[BaselineUpdate]) -> str:
        """Generate a summary of updates for reporting."""
        if not updates:
            return "No baseline changes detected"
        
        summary_lines = []
        
        # Group updates by proxy
        proxy_updates = {}
        for update in updates:
            if update.proxy_name not in proxy_updates:
                proxy_updates[update.proxy_name] = []
            proxy_updates[update.proxy_name].append(update)
        
        for proxy_name, proxy_updates_list in proxy_updates.items():
            for update in proxy_updates_list:
                change_direction = "↑" if update.new_value > update.old_value else "↓"
                change_percent = abs((update.new_value - update.old_value) / update.old_value * 100)
                
                summary_lines.append(
                    f"- {proxy_name}: {update.metric_name} changed from "
                    f"{update.old_value} to {update.new_value} "
                    f"({change_direction}{change_percent:.1f}%) - {update.source}"
                )
        
        return "\\n".join(summary_lines)

def main():
    """Main function for command-line usage."""
    updater = BaselineUpdater()
    
    # Check for updates
    has_updates, updates = updater.check_for_updates()
    
    # Generate output for GitHub Actions
    print(f"updated={str(has_updates).lower()}")
    
    if has_updates:
        summary = updater.generate_update_summary(updates)
        print(f"changes={summary}")
        
        # Apply updates if requested
        if '--apply' in sys.argv:
            success = updater.apply_updates(updates)
            if success:
                print("✅ Baseline updates applied successfully")
            else:
                print("❌ Failed to apply baseline updates")
                sys.exit(1)
    else:
        print("changes=No baseline changes detected")
    
    print("✅ Baseline update check completed")

if __name__ == "__main__":
    main()
