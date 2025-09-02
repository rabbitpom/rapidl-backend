#[allow(non_snake_case)]
pub mod Generate {
    use ::std::fmt;
    use ::std::str;
    use chrono::NaiveDateTime;
    use serde::{Deserialize, Serialize};

    #[derive(Deserialize, Serialize, Debug)]
    pub enum GenerateId {
        MathsMechanics,
        MathsStatistics,
        MathsCore,
    }

    impl fmt::Display for GenerateId {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "{:?}", self)
        }
    }

    impl str::FromStr for GenerateId {
        type Err = String;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            match s {
                "MathsMechanics" => Ok(GenerateId::MathsMechanics),
                "MathsStatistics" => Ok(GenerateId::MathsStatistics),
                "MathsCore" => Ok(GenerateId::MathsCore),
                _ => Err(format!("'{}' is not a valid GenerateId", s)),
            }
        }
    }

    #[derive(Deserialize, Serialize)]
    pub struct SQSBody {
        pub user_id: i64,
        pub job_id: String,
        pub gen_id: GenerateId,
        pub opts: Vec<GenerateOption>,
        pub created_at: NaiveDateTime,
    }

    #[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash)]
    pub enum GenerateOption {
        // MathsMechanics
        SUVAT,
        Momentum,
        Graphs,
        Moments,
        Pullies,
        InclinedSlopes,
        Projectiles,
        Vectors,
        // MathsStatistics
        // Graphs
        Probability,
        HypothesisTesting,
        NormalDistribution,
        BinomialDistribution,
        // MathsCore
        // Graphs
        Algebra,
        Integration,
        Differentiation,
        TrigonometricIdentities,
        CoordinateGeometry,
        SequencesAndSeries,
    }
    impl fmt::Display for GenerateOption {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "{:?}", self)
        }
    }
    impl str::FromStr for GenerateOption {
        type Err = String;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            match s {
                "SUVAT" => Ok(GenerateOption::SUVAT),
                "Momentum" => Ok(GenerateOption::Momentum),
                "Graphs" => Ok(GenerateOption::Graphs),
                "Moments" => Ok(GenerateOption::Moments),
                "Pullies" => Ok(GenerateOption::Pullies),
                "InclinedSlopes" => Ok(GenerateOption::InclinedSlopes),
                "Projectiles" => Ok(GenerateOption::Projectiles),
                "Vectors" => Ok(GenerateOption::Vectors),
                "Probability" => Ok(GenerateOption::Probability),
                "HypothesisTesting" => Ok(GenerateOption::HypothesisTesting),
                "NormalDistribution" => Ok(GenerateOption::NormalDistribution),
                "BinomialDistribution" => Ok(GenerateOption::BinomialDistribution),
                "Algebra" => Ok(GenerateOption::Algebra),
                "Integration" => Ok(GenerateOption::Integration),
                "Differentiation" => Ok(GenerateOption::Differentiation),
                "TrigonometricIdentities" => Ok(GenerateOption::TrigonometricIdentities),
                "CoordinateGeometry" => Ok(GenerateOption::CoordinateGeometry),
                "SequencesAndSeries" => Ok(GenerateOption::SequencesAndSeries),
                _ => Err(format!("'{}' is not a valid GenerateOption", s)),
            }
        }
    }

    pub fn str_to_generation_id<T: AsRef<str>>(id: T) -> Result<GenerateId, String> {
        id.as_ref().parse()
    }
    pub fn str_to_generation_options<T: AsRef<str>>(options: T) -> Result<Vec<GenerateOption>, String> {
        options.as_ref()
            .split(',')
            .map(|x| x.trim().parse::<GenerateOption>())
            .collect()
    }
}

#[allow(non_snake_case)]
pub mod SQSEmail {
    use serde::{Deserialize, Serialize};

    #[derive(Deserialize, Serialize)]
    pub struct SQSBody {
        pub send_bulk: bool,
        pub requires_subscription: bool,
        pub topic: String,
        pub next_token: Option<String>,
        pub template_name: String,
        pub template_data: String,
    }
    pub struct SQSPartialBody {
        pub topic: String,
        pub template_name: String,
        pub template_data: String,
    }
    impl SQSBody {
        pub fn partial_clone(&self) -> SQSPartialBody {
            SQSPartialBody {
                topic: self.topic.clone(),
                template_name: self.template_name.clone(),
                template_data: self.template_data.clone(),
            }
        }
    }
}

#[allow(non_snake_case)]
pub mod SESContacts {
    use ::std::fmt::Display;
    use serde::{Deserialize, Serialize};
    use derive_builder::Builder;

    #[derive(Default, Serialize, Deserialize, Builder)]
    pub struct Response {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[builder(setter(into, strip_option), default)]
        pub is_email_in_mail_list: Option<bool>,
    }

    #[derive(Deserialize, Serialize, Copy, Clone)]
    pub enum TopicType {
        Advertising,
    }

    impl Display for TopicType {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
            match self {
                TopicType::Advertising => write!(f, "marketing-weekly-mail"),
            }
        }
    }
    impl<'a> From<&'a str> for TopicType {
        fn from(s: &'a str) -> Self {
            match s {
                "marketing-weekly-mail" => TopicType::Advertising,
                _ => panic!("Invalid TopicType string: {}", s),
            }
        }
    }

    #[derive(Deserialize, Serialize)]
    pub enum RequestType {
        AddToMailList,
        RemoveFromMailList,
        IsInMailList,
    }

    #[derive(Deserialize, Serialize)]
    pub struct SendIndividual {
        pub template_name: String,
        pub template_data: String,
    }

    #[derive(Deserialize, Serialize)]
    pub enum Command {
        ActionType(RequestType, TopicType),
        SendIndividual(SendIndividual),
        SendIndividualCustomReplyTo(SendIndividual, String),
        SendBulkSubscription(TopicType),
        SendBulk(TopicType),
    }

    #[derive(Deserialize, Serialize)]
    pub struct Request {
        pub commands: Command,
        pub email: String,
    }
}

#[allow(non_snake_case)]
pub mod SESSNS {
    use chrono::{DateTime, Utc};
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub enum NotificationType {
        Bounce,
        Complaint,
        Delivery,
    }

    #[derive(Deserialize)]
    pub struct Mail {
        pub timestamp: DateTime<Utc>,
        #[serde(rename="messageId")]
        pub message_id: String,
        pub source: String,
        #[serde(rename="sourceArn")]
        pub source_arn: String,
        /* ignore everything else */
    }

    #[derive(Deserialize)]
    pub struct Message {
        #[serde(rename="notificationType")]
        pub notification_type: NotificationType,
        pub mail: Mail,
        pub bounce: Option<Bounce>,
        pub complaint: Option<Complaint>,
        pub delivery: Option<Delivery>,
    }

    #[derive(Deserialize)]
    pub struct SQSSNSBody {
        #[serde(rename="MessageId")]
        pub message_id: String,
        #[serde(rename="Message")]
        pub message: String,
    }

    #[derive(Deserialize)]
    pub enum BounceType {
        Undetermined,
        Permanent,
        Transient,
    }
    #[derive(Deserialize)]
    pub enum BounceSubType {
        Undetermined,
        General,
        NoEmail,
        Suppressed,
        OnAccountSuppressionList,
        MailboxFull,
        MessageTooLarge,
        ContentRejected,
        AttachmentRejected,
    }
    #[derive(Deserialize)]
    pub enum ComplaintSubType {
        OnAccountSuppressionList,
    }
    #[derive(Deserialize)]
    pub struct Recipient {
        #[serde(rename="emailAddress")]
        pub email_address: String,
    }

    #[derive(Deserialize)]
    pub struct Bounce {
        #[serde(rename="bounceType")]
        pub bounce_type: BounceType,
        #[serde(rename="bounceSubType")]
        pub bounce_subtype: BounceSubType,
        #[serde(rename="bouncedRecipients")]
        pub bounced_recipients: Vec<Recipient>,
        pub timestamp: DateTime<Utc>,
        #[serde(rename="feedbackId")]
        pub feedback_id: String,
    }

    #[derive(Deserialize)]
    pub struct Complaint {
        #[serde(rename="complainedRecipients")]
        pub complained_recipients: Vec<Recipient>,
        pub timestamp: DateTime<Utc>,
        #[serde(rename="feedbackId")]
        pub feedback_id: String,
        #[serde(rename="complaintSubType")]
        pub complaint_subtype: Option<ComplaintSubType>,
    }

    #[derive(Deserialize)]
    pub struct Delivery {
    }
}

#[allow(non_snake_case)]
pub mod SESEmailBlock {
    use diesel::prelude::*;
    use db_schema::problematicemails;
    use chrono::NaiveDateTime;

    #[derive(Queryable, Selectable, Insertable, Debug)]
    #[diesel(table_name = problematicemails)]
    #[allow(non_snake_case)]
    pub struct EmailBlock {
        pub hash: String,
        pub count: i32,
        pub nextreset: NaiveDateTime,
    }
}

#[allow(non_snake_case)]
pub mod Token {
    use serde::{Deserialize, Serialize};

    #[derive(Deserialize, Serialize)]
    pub struct VerifyToken {
        pub email: String,
        pub username: String,
        pub userid: i64,
    }
}

#[allow(non_snake_case)]
pub mod Ip {
    // Attempt to fetch 'CF-Connecting-IP'
    // Attempt to fetch left-most 'X-Forwarded-For'
    // Attempt to fetch 'X-Real-IP'
    // Attempt to fetch 'Fly-Client-IP'
    // Attempt to fetch 'True-Client-IP'
    // 
    // IF DEVELOPMENT
    // Attempt all of above
    // Attempt to fetch 'Host'

    use ::std::net::{Ipv4Addr, Ipv6Addr};
    use axum::http::HeaderMap;

    const HEADERS: [&'static str; 6] = [
        "cf-connecting-ip",
        "x-forwarded-for",
        "x-real-ip",
        "fly-client-ip",
        "true-client-ip",
        "host",
    ];

    fn try_convert_ipv6(data: &str) -> Option<Ipv6Addr> {
        if let Ok(ipv6) = data.parse::<Ipv6Addr>() {
            return Some(ipv6)
        }
        if let Some((ip_str, _)) = data.split_once(":") {
            if let Ok(ipv4) = ip_str.parse::<Ipv4Addr>() {
                return Some(ipv4.to_ipv6_mapped());
            }
        }
        if let Ok(ipv4) = data.parse::<Ipv4Addr>() {
            return Some(ipv4.to_ipv6_mapped())
        }
        None
    }

    pub fn try_fetch_ipv6(headers: &HeaderMap, developmentMode: bool) -> Option<Ipv6Addr> {
        let iterate_up_to = { if developmentMode { HEADERS.len() } else { HEADERS.len() - 1 } };
        for index in 0..iterate_up_to {
            let header_name = HEADERS[index];
            let header_value = headers.get(header_name);
            if let Some(header_value) = header_value {
                if header_name == "x-forwarded-for" {
                    if let Ok(str_header_value) = header_value.to_str() {
                        let split_ips = str_header_value.split(',').collect::<Vec<&str>>();
                        if split_ips.is_empty() {
                            continue
                        }
                        let left_most_ip = split_ips[0];
                        if let Some(ipv6) = try_convert_ipv6(left_most_ip) {
                            return Some(ipv6)
                        }
                    }
                    continue
                }
                // https://superuser.com/questions/381022/how-many-characters-can-an-ip-address-be
                if header_value.len() > 62 {
                    continue
                }
                if let Ok(str_header_value) = header_value.to_str() {
                    if let Some(ipv6) = try_convert_ipv6(str_header_value) {
                        return Some(ipv6)
                    } 
                } 
            }
        }
        if developmentMode {
            return Some(Ipv6Addr::new(0,0,0,0,0,0,0,1))
        }
        None
    }
}
