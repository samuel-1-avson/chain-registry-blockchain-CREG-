//! Risk Model for Package Insurance
//!
//! Calculates risk scores based on package metrics and historical data.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

/// Risk factors for a package
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFactor {
    /// Factor name
    pub name: String,
    /// Weight (0-1)
    pub weight: f64,
    /// Score contribution (0-100)
    pub score: f64,
}

impl RiskFactor {
    /// Create new risk factor
    pub fn new(name: &str, weight: f64, score: f64) -> Self {
        Self {
            name: name.to_string(),
            weight: weight.clamp(0.0, 1.0),
            score: score.clamp(0.0, 100.0),
        }
    }
    
    /// Calculate weighted contribution
    pub fn contribution(&self) -> f64 {
        self.weight * self.score
    }
}

/// Package metrics for risk assessment
#[derive(Debug, Clone, Default)]
pub struct PackageMetrics {
    /// Package age in days
    pub age_days: u32,
    /// Number of dependencies
    pub dependency_count: u32,
    /// Number of reverse dependencies (dependents)
    pub dependent_count: u32,
    /// Number of known vulnerabilities
    pub vulnerability_count: u32,
    /// Has security audit
    pub has_audit: bool,
    /// Last update timestamp
    pub last_update_days: u32,
    /// Code complexity score
    pub complexity_score: f64,
    /// Download count (popularity)
    pub download_count: u64,
    /// Number of maintainers
    pub maintainer_count: u32,
    /// Is deprecated
    pub is_deprecated: bool,
    /// Historical incident count
    pub incident_count: u32,
}

impl PackageMetrics {
    /// Calculate age risk (older = lower risk)
    pub fn age_risk(&self) -> f64 {
        match self.age_days {
            0..=30 => 80.0,     // New package - high risk
            31..=90 => 60.0,    // Young package - medium-high risk
            91..=365 => 40.0,   // Established - medium risk
            366..=730 => 20.0,  // Mature - low risk
            _ => 10.0,          // Very mature - very low risk
        }
    }
    
    /// Calculate dependency risk (more deps = higher risk)
    pub fn dependency_risk(&self) -> f64 {
        match self.dependency_count {
            0..=5 => 10.0,
            6..=20 => 30.0,
            21..=50 => 50.0,
            51..=100 => 70.0,
            _ => 85.0,
        }
    }
    
    /// Calculate popularity risk (more dependents = higher impact if compromised)
    pub fn popularity_risk(&self) -> f64 {
        match self.dependent_count {
            0..=10 => 10.0,
            11..=100 => 30.0,
            101..=1000 => 50.0,
            1001..=10000 => 70.0,
            _ => 90.0,
        }
    }
    
    /// Calculate vulnerability risk
    pub fn vulnerability_risk(&self) -> f64 {
        let base_risk = match self.vulnerability_count {
            0 => 0.0,
            1 => 30.0,
            2..=5 => 60.0,
            _ => 90.0,
        };
        
        // Adjust based on time since last update
        let recency_factor = if self.last_update_days > 365 {
            1.5 // Not maintained
        } else if self.last_update_days > 90 {
            1.2 // Stale
        } else {
            1.0
        };
        
        (base_risk * recency_factor).min(100.0)
    }
    
    /// Calculate maintenance risk
    pub fn maintenance_risk(&self) -> f64 {
        let mut risk = 0.0;
        
        // Few maintainers
        if self.maintainer_count == 0 {
            risk += 50.0;
        } else if self.maintainer_count == 1 {
            risk += 30.0;
        }
        
        // Not updated recently
        if self.last_update_days > 365 {
            risk += 40.0;
        } else if self.last_update_days > 180 {
            risk += 20.0;
        }
        
        // Deprecated
        if self.is_deprecated {
            risk += 80.0;
        }
        
        risk.min(100.0)
    }
    
    /// Calculate complexity risk
    pub fn complexity_risk(&self) -> f64 {
        self.complexity_score.clamp(0.0, 100.0)
    }
    
    /// Calculate audit benefit
    pub fn audit_benefit(&self) -> f64 {
        if self.has_audit {
            -20.0 // Reduces risk
        } else {
            0.0
        }
    }
    
    /// Calculate historical incident risk
    pub fn incident_risk(&self) -> f64 {
        match self.incident_count {
            0 => 0.0,
            1 => 30.0,
            2..=3 => 60.0,
            _ => 90.0,
        }
    }
}

/// Risk model for insurance pricing
#[derive(Debug, Clone)]
pub struct RiskModel {
    /// Factor weights
    weights: HashMap<String, f64>,
    /// Historical data
    historical_incidents: HashMap<String, u32>,
}

impl Default for RiskModel {
    fn default() -> Self {
        let mut weights = HashMap::new();
        weights.insert("age".to_string(), 0.15);
        weights.insert("dependencies".to_string(), 0.15);
        weights.insert("popularity".to_string(), 0.20);
        weights.insert("vulnerabilities".to_string(), 0.25);
        weights.insert("maintenance".to_string(), 0.15);
        weights.insert("complexity".to_string(), 0.05);
        weights.insert("incidents".to_string(), 0.05);
        
        Self {
            weights,
            historical_incidents: HashMap::new(),
        }
    }
}

impl RiskModel {
    /// Create custom risk model
    pub fn with_weights(weights: HashMap<String, f64>) -> Self {
        Self {
            weights,
            historical_incidents: HashMap::new(),
        }
    }
    
    /// Calculate risk score for a package
    pub fn calculate_score(&self, package: &str) -> Result<f64, super::InsuranceError> {
        // In production, fetch metrics from database/API
        // For now, generate deterministic pseudo-random score
        let metrics = self.estimate_metrics(package);
        
        let score = self.calculate_from_metrics(&metrics);
        
        debug!("Risk score for {}: {:.2}", package, score);
        
        Ok(score)
    }
    
    /// Calculate risk from metrics
    pub fn calculate_from_metrics(&self, metrics: &PackageMetrics) -> f64 {
        let mut factors: Vec<(String, f64, f64)> = vec![
            ("age".to_string(), self.weights["age"], metrics.age_risk()),
            ("dependencies".to_string(), self.weights["dependencies"], metrics.dependency_risk()),
            ("popularity".to_string(), self.weights["popularity"], metrics.popularity_risk()),
            ("vulnerabilities".to_string(), self.weights["vulnerabilities"], metrics.vulnerability_risk()),
            ("maintenance".to_string(), self.weights["maintenance"], metrics.maintenance_risk()),
            ("complexity".to_string(), self.weights["complexity"], metrics.complexity_risk()),
            ("incidents".to_string(), self.weights["incidents"], metrics.incident_risk()),
        ];
        
        // Calculate weighted sum
        let mut total_score = 0.0;
        let mut total_weight = 0.0;
        
        for (_, weight, score) in &factors {
            total_score += weight * score;
            total_weight += weight;
        }
        
        // Add audit benefit
        total_score += metrics.audit_benefit();
        
        // Normalize and clamp
        let final_score = (total_score / total_weight).clamp(0.0, 100.0);
        
        final_score
    }
    
    /// Estimate metrics from package name (for demo)
    fn estimate_metrics(&self, package: &str) -> PackageMetrics {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        package.hash(&mut hasher);
        let hash = hasher.finish();
        
        // Generate pseudo-random metrics from hash
        PackageMetrics {
            age_days: ((hash >> 48) % 1000) as u32,
            dependency_count: ((hash >> 40) % 50) as u32,
            dependent_count: ((hash >> 32) % 5000) as u32,
            vulnerability_count: ((hash >> 24) % 5) as u32,
            has_audit: (hash >> 16) % 10 == 0, // 10% have audits
            last_update_days: ((hash >> 8) % 400) as u32,
            complexity_score: ((hash % 100) as f64).clamp(0.0, 100.0),
            download_count: ((hash >> 32) % 1000000) as u64,
            maintainer_count: ((hash >> 48) % 5 + 1) as u32,
            is_deprecated: hash % 100 == 0, // 1% deprecated
            incident_count: ((hash >> 56) % 3) as u32,
        }
    }
    
    /// Add historical incident
    pub fn add_incident(&mut self, package: &str) {
        *self.historical_incidents.entry(package.to_string()).or_insert(0) += 1;
    }
    
    /// Get incident count
    pub fn get_incidents(&self, package: &str) -> u32 {
        self.historical_incidents.get(package).copied().unwrap_or(0)
    }
    
    /// Get risk category
    pub fn risk_category(score: f64) -> RiskCategory {
        match score as u32 {
            0..=20 => RiskCategory::VeryLow,
            21..=40 => RiskCategory::Low,
            41..=60 => RiskCategory::Medium,
            61..=80 => RiskCategory::High,
            _ => RiskCategory::VeryHigh,
        }
    }
    
    /// Get premium rate for risk category
    pub fn premium_rate(category: RiskCategory) -> f64 {
        match category {
            RiskCategory::VeryLow => 0.005,  // 0.5%
            RiskCategory::Low => 0.01,       // 1%
            RiskCategory::Medium => 0.025,   // 2.5%
            RiskCategory::High => 0.05,      // 5%
            RiskCategory::VeryHigh => 0.10,  // 10%
        }
    }
}

/// Risk categories
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskCategory {
    VeryLow,
    Low,
    Medium,
    High,
    VeryHigh,
}

impl std::fmt::Display for RiskCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskCategory::VeryLow => write!(f, "Very Low"),
            RiskCategory::Low => write!(f, "Low"),
            RiskCategory::Medium => write!(f, "Medium"),
            RiskCategory::High => write!(f, "High"),
            RiskCategory::VeryHigh => write!(f, "Very High"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_risk_factor() {
        let factor = RiskFactor::new("test", 0.5, 80.0);
        assert_eq!(factor.contribution(), 40.0);
    }

    #[test]
    fn test_package_metrics_age() {
        let young = PackageMetrics { age_days: 10, ..Default::default() };
        let old = PackageMetrics { age_days: 500, ..Default::default() };
        
        assert!(young.age_risk() > old.age_risk());
    }

    #[test]
    fn test_risk_score_calculation() {
        let model = RiskModel::default();
        
        let metrics = PackageMetrics {
            age_days: 30,
            vulnerability_count: 2,
            is_deprecated: false,
            ..Default::default()
        };
        
        let score = model.calculate_from_metrics(&metrics);
        
        // Should be medium-high risk
        assert!(score > 30.0 && score < 80.0);
    }

    #[test]
    fn test_risk_category() {
        assert_eq!(RiskModel::risk_category(10.0), RiskCategory::VeryLow);
        assert_eq!(RiskModel::risk_category(35.0), RiskCategory::Low);
        assert_eq!(RiskModel::risk_category(55.0), RiskCategory::Medium);
        assert_eq!(RiskModel::risk_category(75.0), RiskCategory::High);
        assert_eq!(RiskModel::risk_category(95.0), RiskCategory::VeryHigh);
    }

    #[test]
    fn test_premium_rates() {
        let very_low = RiskModel::premium_rate(RiskCategory::VeryLow);
        let very_high = RiskModel::premium_rate(RiskCategory::VeryHigh);
        
        assert!(very_high > very_low);
        assert_eq!(very_low, 0.005);
        assert_eq!(very_high, 0.10);
    }
}
