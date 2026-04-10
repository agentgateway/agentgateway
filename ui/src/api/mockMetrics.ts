/**
 * LLM metrics
 */
export const mockTokenUsageByModelData = [
    {
      label: "gpt-4",
      value: 300,
      color: '#9554d8',
      tooltipData: [
        {
          title: "Model",
          rows: [
            { value: "gpt-4" },
          ],
        },
        {
          title: "Token Usage",
          rows: [
            { label: "Input Tokens", value: 100 },
            { label: "Output Tokens", value: 200 },
            { label: "Total Tokens", value: 300 },
          ],
        },
        {
          title: "Request Count",
          rows: [
            { value: 50 },
          ],
        },
      ],
    },
    {
      label: 'gpt-3.5-turbo',
      value: 400,
      color: '#5434C7',
      tooltipData: [
        {
          title: "Model",
          rows: [
            { value: "gpt-3.5-turbo" },
          ],
        },
        {
          title: "Token Usage",
          rows: [
            { label: "Input Tokens", value: 150 },
            { label: "Output Tokens", value: 250 },
            { label: "Total Tokens", value: 400 },
          ],
        },
        {
          title: "Request Count",
          rows: [
            { value: 100 },
          ],
        },
      ],
    },
  ];
  
  export const mockRequestThroughputLabels = ['2026-03-31', '2026-04-01', '2026-04-02', '2026-04-03', '2026-04-04', '2026-04-05', '2026-04-06']; 
  export const mockRequestThroughputDataset = [
    {
      label: 'Request Throughput',
      data: [0, 0, 150, 250, 0, 350, 400],
      borderColor: '#9554d8',
    },
  ];
  
  export const mockLLMLatencyData = { 
    labels: ["p50", "p75", "p90", "p95", "p99"],
    datasets: [{
      label: "Latency (ms)",
      data: [12, 18, 35, 52, 120],
      backgroundColor: "#9554d8",
    }],
  };
  
  export const mockPerModelLatencyLabels = ['gpt-4', 'gpt-3.5-turbo'];
  export const mockPerModelLatencyDatasets = [
    { label: 'p50', data: [18, 12], backgroundColor: '#9554d8' },
    { label: 'p90', data: [45, 28], backgroundColor: '#5434C7' },
    { label: 'p99', data: [130, 75], backgroundColor: '#3a238a' },
  ];
  
  export const mockPerModelThroughputLabels = ['2026-03-31', '2026-04-01', '2026-04-02', '2026-04-03', '2026-04-04', '2026-04-05', '2026-04-06'];
  export const mockPerModelThroughputDatasets = [
    { label: 'gpt-4', data: [0, 0, 80, 120, 0, 200, 210], borderColor: '#9554d8' },
    { label: 'gpt-3.5-turbo', data: [0, 0, 70, 130, 0, 150, 190], borderColor: '#5434C7' },
  ];
  
  export const mockLLMErrorRateData = {
    labels: ["12:00", "12:05", "12:10", "12:15", "12:20"],
    datasets: [
        { label: "gpt-4", data: [0.02, 0.05, 0.03, 0.08, 0.04], borderColor: "#9554d8" },
        { label: "gpt-3.5-turbo", data: [0.01, 0.01, 0.02, 0.01, 0.03], borderColor: "#5434C7" },
    ]
  };

/**
 * MCP metrics
 */
export const mockToolCallCountsData = [
  { label: "fetch", value: 350, color: '#9554d8', tooltipData: [{ title: "Call Count", rows: [{ value: 350}]}] },
  { label: "get_weather", value: 280, color: '#5434C7', tooltipData: [{ title: "Call Count", rows: [{ value: 280}]}] },
  { label: "execute_command", value: 195, color: '#3a238a', tooltipData: [{ title: "Call Count", rows: [{ value: 195}]}]  },
  { label: "get_stock_price", value: 142, color: '#7c3aed', tooltipData: [{ title: "Call Count", rows: [{ value: 142}]}] },
];
export const mockMCPErrorRateData = {
  labels: ['2026-04-03', '2026-04-04', '2026-04-05', '2026-04-06', '2026-04-07', '2026-04-08', '2026-04-09'],
  datasets: [
    { label: 'fetch', data: [0.02, 0.01, 0.03, 0.01, 0.02, 0.01, 0.01], borderColor: '#9554d8' },
    { label: 'get_weather', data: [0.05, 0.04, 0.06, 0.03, 0.04, 0.05, 0.02], borderColor: '#5434C7' },
    { label: 'execute_command', data: [0.10, 0.12, 0.08, 0.15, 0.09, 0.11, 0.07], borderColor: '#3a238a' },
    { label: 'get_stock_price', data: [0.01, 0.02, 0.01, 0.03, 0.02, 0.01, 0.02], borderColor: '#7c3aed' },
  ]
};
export const mockMCPLatencyDistributionLabels = ['fetch', 'get_weather', 'execute_command', 'get_stock_price'];
export const mockMCPLatencyDistributionDatasets = [
  { label: 'p50', data: [12, 18, 45, 8], backgroundColor: '#9554d8' },
  { label: 'p75', data: [20, 28, 75, 14], backgroundColor: '#5434C7' },
  { label: 'p90', data: [28, 42, 120, 22], backgroundColor: '#3a238a' },
  { label: 'p95', data: [45, 65, 200, 35], backgroundColor: '#7c3aed' },
  { label: 'p99', data: [85, 150, 380, 65], backgroundColor: '#6d28d9' },
];

export const mockPerTargetLatencyLabels = ['fetch', 'get_weather', 'execute_command', 'get_stock_price'];
export const mockPerTargetLatencyDatasets = [
  { label: 'p50', data: [12, 18, 45, 8], backgroundColor: '#9554d8' },
  { label: 'p90', data: [28, 42, 120, 22], backgroundColor: '#5434C7' },
  { label: 'p99', data: [85, 150, 380, 65], backgroundColor: '#3a238a' },
];

export const mockPerTargetCallsLabels = ['2026-04-03', '2026-04-04', '2026-04-05', '2026-04-06', '2026-04-07', '2026-04-08', '2026-04-09'];
export const mockPerTargetCallsDatasets = [
  { label: 'fetch', data: [45, 52, 38, 61, 55, 70, 85], borderColor: '#9554d8' },
  { label: 'get_weather', data: [28, 31, 29, 35, 42, 38, 45], borderColor: '#5434C7' },
  { label: 'execute_command', data: [15, 18, 22, 25, 28, 32, 40], borderColor: '#3a238a' },
  { label: 'get_stock_price', data: [12, 14, 16, 18, 20, 24, 30], borderColor: '#7c3aed' },
];
  
/**
 * Traffic metrics
 */
export const mockRequestCountByRouteData = [
  { label: "/api/users", value: 1250, color: '#9554d8', tooltipData: [
    { title: "Route", rows: [{ value: "/api/users" }] },
    { title: "Request Count", rows: [{ value: 1250 }] },
  ]},
  { label: "/api/products", value: 890, color: '#5434C7', tooltipData: [
    { title: "Route", rows: [{ value: "/api/products" }] },
    { title: "Request Count", rows: [{ value: 890 }] },
  ]},
  { label: "/api/orders", value: 620, color: '#3a238a', tooltipData: [
    { title: "Route", rows: [{ value: "/api/orders" }] },
    { title: "Request Count", rows: [{ value: 620 }] },
  ]},
  { label: "/api/auth", value: 450, color: '#7c3aed', tooltipData: [
    { title: "Route", rows: [{ value: "/api/auth" }] },
    { title: "Request Count", rows: [{ value: 450 }] },
  ]},
];

export const mockTrafficLatencyDistributionLabels = ['/api/users', '/api/products', '/api/orders', '/api/auth'];
export const mockTrafficLatencyDistributionDatasets = [
  { label: 'p50', data: [12, 18, 45, 8], backgroundColor: '#9554d8' },
  { label: 'p75', data: [20, 28, 75, 14], backgroundColor: '#5434C7' },
  { label: 'p90', data: [28, 42, 120, 22], backgroundColor: '#3a238a' },
  { label: 'p95', data: [45, 65, 200, 35], backgroundColor: '#7c3aed' },
  { label: 'p99', data: [85, 150, 380, 65], backgroundColor: '#6d28d9' },
];

export const mockTrafficErrorRateLabels = ['2026-04-03', '2026-04-04', '2026-04-05', '2026-04-06'];
export const mockTrafficErrorRateDatasets = [
  { label: '/api/users', data: [0.02, 0.01, 0.03, 0.01], borderColor: '#9554d8' },
  { label: '/api/products', data: [0.05, 0.04, 0.06, 0.03], borderColor: '#5434C7' },
  { label: '/api/orders', data: [0.10, 0.12, 0.08, 0.15], borderColor: '#3a238a' },
  { label: '/api/auth', data: [0.01, 0.02, 0.01, 0.03], borderColor: '#7c3aed' },
];

export const mockTrafficPerRouteLatencyLabels = ['/api/users', '/api/products', '/api/orders', '/api/auth'];
export const mockTrafficPerRouteLatencyDatasets = [
  { label: 'p50', data: [15, 22, 18, 12], backgroundColor: '#9554d8' },
  { label: 'p90', data: [45, 65, 52, 35], backgroundColor: '#5434C7' },
  { label: 'p99', data: [120, 180, 150, 95], backgroundColor: '#3a238a' },
];

export const mockTrafficPerRouteVolumeLabels = ['2026-04-03', '2026-04-04', '2026-04-05', '2026-04-06', '2026-04-07', '2026-04-08', '2026-04-09'];
export const mockTrafficPerRouteVolumeDatasets = [
  { label: '/api/users', data: [980, 1050, 1120, 1200, 1150, 1280, 1250], borderColor: '#9554d8' },
  { label: '/api/products', data: [750, 780, 820, 850, 880, 890, 890], borderColor: '#5434C7' },
  { label: '/api/orders', data: [520, 540, 580, 600, 620, 615, 620], borderColor: '#3a238a' },
  { label: '/api/auth', data: [380, 400, 420, 440, 450, 450, 450], borderColor: '#7c3aed' },
];