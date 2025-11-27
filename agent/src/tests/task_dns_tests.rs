//! Tests for DNS query task implementation

use crate::task_dns::{convert_record_type, execute_dns_query};
use hickory_client::proto::rr::RecordType as ClientRecordType;
use shared::config::{DnsQueryParams, DnsRecordType};

#[tokio::test]
async fn test_dns_query_google() {
    let params = DnsQueryParams {
        server: "8.8.8.8:53".to_string(),
        domain: "google.com".to_string(),
        record_type: DnsRecordType::A,
        timeout_seconds: 5,
        expected_ip: None,
        target_id: None,
    };

    let result = execute_dns_query(&params).await;
    assert!(result.is_ok());

    let metric = result.unwrap();
    assert_eq!(metric.domain_queried, "google.com");
    if metric.success {
        assert!(metric.query_time_ms.is_some());
        assert!(metric.record_count.is_some());
        assert!(metric.resolved_addresses.is_some());
    }
}

#[test]
fn test_record_type_conversion() {
    assert_eq!(convert_record_type(&DnsRecordType::A), ClientRecordType::A);
    assert_eq!(
        convert_record_type(&DnsRecordType::AAAA),
        ClientRecordType::AAAA
    );
    assert_eq!(
        convert_record_type(&DnsRecordType::MX),
        ClientRecordType::MX
    );
    assert_eq!(
        convert_record_type(&DnsRecordType::CNAME),
        ClientRecordType::CNAME
    );
    assert_eq!(
        convert_record_type(&DnsRecordType::TXT),
        ClientRecordType::TXT
    );
    assert_eq!(
        convert_record_type(&DnsRecordType::NS),
        ClientRecordType::NS
    );
}
